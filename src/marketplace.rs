use crate::archive::{DecodedPack, MAX_ARCHIVE, decode_bundle};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bedrock_services::{minecraft::Minecraft, playfab::EntityKind};
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;
use zeroize::Zeroizing;

mod browse;
mod purchase;
mod version;
pub(crate) use browse::published as published_item;
pub use browse::{
    CatalogItem, CatalogPage, CatalogQuery, Ownership, PAGE_SIZE, browse, receipt_ownership,
};
pub(crate) use purchase::{PurchaseAttempt, submit_purchase};
pub use purchase::{PurchaseQuote, TransactionFailure, minecoin_balance};

const INVENTORY: &str = "https://entitlements.mktpl.minecraft-services.net/api/v1.0/player/inventory?includeReceipt=true";
const CATALOG: &str = "https://20ca2.playfabapi.com/Catalog/";
pub const POTATO: &str = "790be090-9bce-4870-85e7-cd868402c3e5";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Addon {
    pub id: Uuid,
    pub name: String,
    pub creator: String,
    pub owned: bool,
    pub packs: Vec<PackIdentity>,
    pub price: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackIdentity {
    pub uuid: Uuid,
    pub version: [u32; 3],
    pub kind: String,
}

#[expect(
    clippy::disallowed_methods,
    reason = "The shared service client sets connect, read, and total timeouts"
)]
pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(180))
        .user_agent("BedrockFoundry/0.1.0")
        .build()?)
}
pub async fn bounded_response(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        response.status().is_success(),
        "Service returned HTTP {}",
        response.status().as_u16()
    );
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "Download exceeds the size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        ensure!(
            bytes
                .len()
                .checked_add(chunk.len())
                .is_some_and(|n| n <= limit),
            "Download exceeds the size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn json_response(response: Response) -> Result<Value> {
    Ok(serde_json::from_slice(
        &bounded_response(response, 8 * 1024 * 1024).await?,
    )?)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Receipt {
    entity_id: String,
    #[serde(rename = "ReceiptData")]
    data: ReceiptData,
    entitlement_receipts: Vec<Entitlement>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ReceiptData {
    device_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Entitlement {
    pack_id: Uuid,
    content_key: Option<String>,
    amount: u64,
    expiration_date: Option<String>,
    start_date: Option<String>,
    #[serde(default, deserialize_with = "null_offer_ids")]
    offer_ids: Vec<Uuid>,
}

fn null_offer_ids<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<Uuid>, D::Error> {
    Ok(Option::<Vec<Uuid>>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests;

fn decode_receipt(encoded: &str) -> Result<Receipt> {
    ensure!(
        encoded.len() <= 12 * 1024 * 1024,
        "Entitlement receipt exceeds the size limit"
    );
    let bytes = Zeroizing::new(
        STANDARD
            .decode(encoded)
            .context("Invalid entitlement receipt encoding")?,
    );
    ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "Entitlement receipt exceeds the size limit"
    );
    serde_json::from_slice(&bytes).context("Invalid entitlement receipt")
}

fn active(entitlement: &Entitlement, now: OffsetDateTime) -> Result<bool> {
    if entitlement.amount == 0 {
        return Ok(false);
    }
    if let Some(date) = entitlement
        .expiration_date
        .as_ref()
        .filter(|d| !d.starts_with("0001-"))
        && OffsetDateTime::parse(date, &Rfc3339).context("Invalid entitlement expiry")? <= now
    {
        return Ok(false);
    }
    if let Some(date) = entitlement
        .start_date
        .as_ref()
        .filter(|d| !d.starts_with("0001-"))
        && OffsetDateTime::parse(date, &Rfc3339).context("Invalid entitlement start date")? > now
    {
        return Ok(false);
    }
    Ok(true)
}

pub fn receipt_keys(
    encoded: &str,
    now: OffsetDateTime,
) -> Result<HashMap<Uuid, Zeroizing<Vec<u8>>>> {
    let receipt = decode_receipt(encoded)?;
    let entity = receipt.entity_id.as_bytes();
    let device = receipt.data.device_id.as_bytes();
    ensure!(
        entity.len() >= 32 && device.len() >= 32,
        "Entitlement receipt has an invalid identity"
    );
    let mut result = HashMap::new();
    for entitlement in receipt.entitlement_receipts {
        if !active(&entitlement, now)? {
            continue;
        }
        let Some(encoded) = entitlement.content_key.filter(|s| !s.is_empty()) else {
            continue;
        };
        let wrapped = Zeroizing::new(
            STANDARD
                .decode(encoded)
                .context("Invalid wrapped entitlement key")?,
        );
        ensure!(wrapped.len() == 64, "Invalid wrapped entitlement key size");
        let key = (0..32)
            .map(|i| wrapped[2 * i] ^ entity[i] ^ device[i])
            .collect();
        result.insert(entitlement.pack_id, Zeroizing::new(key));
    }
    Ok(result)
}

async fn inventory(
    services: &mut Minecraft,
    http: &Client,
) -> Result<HashMap<Uuid, Zeroizing<Vec<u8>>>> {
    receipt_keys(
        &inventory_receipt(services, http).await?,
        OffsetDateTime::now_utc(),
    )
}

async fn inventory_receipt(services: &mut Minecraft, http: &Client) -> Result<Zeroizing<String>> {
    let auth = services.authorize().await?;
    let response = http
        .get(INVENTORY)
        .header("Authorization", auth.header_value()?)
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        services.reject(&auth);
    }
    let inventory = json_response(response).await?;
    Ok(Zeroizing::new(
        inventory["result"]["receipt"]
            .as_str()
            .context("Inventory has no receipt")?
            .to_owned(),
    ))
}

async fn catalog(
    services: &mut Minecraft,
    http: &Client,
    method: &str,
    body: Value,
    master: bool,
) -> Result<Value> {
    let entity = services
        .entity(if master {
            EntityKind::MasterPlayerAccount
        } else {
            EntityKind::TitlePlayerAccount
        })
        .await?;
    let response = http
        .post(format!("{CATALOG}{method}"))
        .header("X-EntityToken", entity.header_value()?)
        .json(&body)
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    json_response(response)
        .await
        .with_context(|| format!("Marketplace catalog {method} failed"))
}
pub fn parse_addon<S: std::hash::BuildHasher>(
    value: &Value,
    owned: &HashSet<Uuid, S>,
) -> Result<Option<Addon>> {
    if !value["Tags"]
        .as_array()
        .is_some_and(|tags| tags.iter().any(|tag| tag.as_str() == Some("addon")))
    {
        return Ok(None);
    }
    let Some(identities) = value["DisplayProperties"]["packIdentity"].as_array() else {
        return Ok(None);
    };
    if !identities
        .iter()
        .any(|p| p["type"].as_str() == Some("behaviorpack"))
    {
        return Ok(None);
    }
    let mut packs = Vec::new();
    for identity in identities {
        let kind = identity["type"]
            .as_str()
            .context("Catalog pack has no type")?;
        if !matches!(kind, "behaviorpack" | "resourcepack") {
            continue;
        }
        packs.push(PackIdentity {
            uuid: identity["uuid"]
                .as_str()
                .context("Catalog pack has no UUID")?
                .parse()?,
            version: version::parse(
                identity["version"]
                    .as_str()
                    .context("Catalog pack has no version")?,
            )?,
            kind: kind.into(),
        });
    }
    ensure!(!packs.is_empty(), "Catalog add-on has no supported packs");
    let name = value["Title"]["en-US"]
        .as_str()
        .or_else(|| value["Title"]["NEUTRAL"].as_str())
        .or_else(|| value["Title"]["neutral"].as_str())
        .context("Catalog add-on has no title")?;
    Ok(Some(Addon {
        id: value["Id"]
            .as_str()
            .context("Catalog add-on has no ID")?
            .parse()?,
        name: name.into(),
        creator: value["DisplayProperties"]["creatorName"]
            .as_str()
            .unwrap_or("Unknown creator")
            .into(),
        owned: packs.iter().all(|p| owned.contains(&p.uuid)),
        packs,
        price: value["DisplayProperties"]["price"].as_u64(),
    }))
}

pub async fn list(services: &mut Minecraft, http: &Client) -> Result<Vec<Addon>> {
    let keys = inventory(services, http).await?;
    let owned: HashSet<_> = keys.keys().copied().collect();
    let mut ids: Vec<_> = owned.iter().collect();
    ids.sort_unstable();
    let mut items = BTreeMap::new();
    for chunk in ids.chunks(6) {
        let filter = chunk
            .iter()
            .map(|id| format!("tags/any(t:t eq '{id}')"))
            .collect::<Vec<_>>()
            .join(" or ");
        for page in 0..20 {
            let body = json!({"Top":50,"Skip":page*50,"Filter":filter,"OrderBy":"id asc"});
            let response = catalog(services, http, "Search", body, true).await?;
            let records = response["data"]["Items"]
                .as_array()
                .context("Catalog search has no items")?;
            for item in records {
                if let Some(addon) = parse_addon(item, &owned)?.filter(|a| a.owned) {
                    items.insert(addon.id, addon);
                }
            }
            let total = response["data"]["Count"]
                .as_u64()
                .context("Catalog search has no total count")?;
            if records.is_empty() || page * 50 + records.len() as u64 >= total {
                break;
            }
            ensure!(
                page < 19,
                "Catalog returned too many pages. Retry the library refresh."
            );
        }
    }
    let mut addons: Vec<_> = items.into_values().collect();
    addons.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(addons)
}

pub async fn free_sample(services: &mut Minecraft, http: &Client) -> Result<Addon> {
    let owned: HashSet<_> = inventory(services, http).await?.into_keys().collect();
    let published = catalog(
        services,
        http,
        "GetPublishedItem",
        json!({"ItemId":POTATO}),
        false,
    )
    .await?;
    parse_addon(&published["data"]["Item"], &owned)?.context("The free sample is not an add-on")
}

pub async fn claim_free(services: &mut Minecraft, http: &Client, id: Uuid) -> Result<()> {
    let published = catalog(
        services,
        http,
        "GetPublishedItem",
        json!({"ItemId":id}),
        false,
    )
    .await?;
    let item = &published["data"]["Item"];
    ensure!(
        item["Id"].as_str() == Some(&id.to_string())
            && item["DisplayProperties"]["price"].as_u64() == Some(0)
            && item["DisplayProperties"]["purchasable"].as_bool() == Some(true),
        "This offer is not currently free. No purchase was made."
    );
    ensure!(
        parse_addon(item, &HashSet::new())?.is_some(),
        "This offer is not an add-on"
    );
    let auth = services.authorize().await?;
    let response = http.post("https://entitlements.mktpl.minecraft-services.net/api/v1.0/transaction/virtual").header("Authorization",auth.header_value()?).json(&json!({"CustomTags":{"CorrelationId":Uuid::new_v4()},"OfferId":id,"StoreId":"","VirtualCurrency":{"Amount":"0","Type":"Minecoin"}})).send().await.map_err(reqwest::Error::without_url)?;
    let result = json_response(response).await?;
    ensure!(
        result["result"]["offerId"].as_str() == Some(&id.to_string()),
        "The claim returned a different offer. Refresh your library before retrying."
    );
    Ok(())
}

pub fn validate_asset_url(url: &str) -> Result<Url> {
    let url = Url::parse(url)?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url.port_or_known_default() == Some(443),
        "Catalog returned an invalid asset URL"
    );
    let host = url.host_str().context("Asset URL has no host")?;
    let allowed = host
        .strip_prefix("xforgeassets")
        .and_then(|s| s.strip_suffix(".xboxlive.com"))
        .or_else(|| {
            host.strip_prefix("content")
                .and_then(|s| s.strip_suffix(".prod.catalog.playfab.com"))
        })
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
    ensure!(allowed, "Catalog asset host is not supported");
    Ok(url)
}

/// Selects the add-on download from a published catalog item and validates its URL.
pub fn download_url(item: &Value) -> Result<Url> {
    let content = item["Contents"]
        .as_array()
        .context("Catalog has no downloadable content")?
        .iter()
        .find(|c| c["Type"].as_str() == Some("resourcebinary"))
        .context("No supported add-on download is available")?;
    validate_asset_url(
        content["Url"]
            .as_str()
            .context("Catalog download has no URL")?,
    )
}

pub async fn download(
    services: &mut Minecraft,
    http: &Client,
    addon: &Addon,
) -> Result<Vec<DecodedPack>> {
    let keys = inventory(services, http).await?;
    ensure!(
        addon.packs.iter().all(|p| keys.contains_key(&p.uuid)),
        "This account does not own all packs in this add-on"
    );
    let published = catalog(
        services,
        http,
        "GetPublishedItem",
        json!({"ItemId":addon.id}),
        false,
    )
    .await?;
    let item = &published["data"]["Item"];
    let current = parse_addon(item, &keys.keys().copied().collect::<HashSet<_>>())?
        .context("Catalog item is no longer an add-on")?;
    ensure!(
        current
            .packs
            .iter()
            .map(|p| (p.uuid, p.version))
            .collect::<HashMap<_, _>>()
            == addon.packs.iter().map(|p| (p.uuid, p.version)).collect(),
        "The add-on changed. Refresh your library before downloading."
    );
    let url = download_url(item)?;
    let response = http
        .get(url)
        .header("User-Agent", "libhttpclient/1.0.0.0")
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    let bytes = bounded_response(response, MAX_ARCHIVE)
        .await
        .context("Marketplace asset download failed")?;
    let expected: HashMap<_, _> = addon.packs.iter().map(|p| (p.uuid, p.version)).collect();
    if bytes.is_empty() {
        bail!("Marketplace returned an empty add-on download");
    }
    decode_bundle(&bytes, &keys, &expected)
}
