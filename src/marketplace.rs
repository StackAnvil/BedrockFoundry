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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackIdentity {
    pub uuid: Uuid,
    pub version: [u32; 3],
    pub kind: String,
}

pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
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
    while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
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
    receipt_data: ReceiptData,
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
}

pub fn receipt_keys(
    encoded: &str,
    now: OffsetDateTime,
) -> Result<HashMap<Uuid, Zeroizing<Vec<u8>>>> {
    let bytes = Zeroizing::new(
        STANDARD
            .decode(encoded)
            .context("Invalid entitlement receipt encoding")?,
    );
    ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "Entitlement receipt exceeds the size limit"
    );
    let receipt: Receipt = serde_json::from_slice(&bytes).context("Invalid entitlement receipt")?;
    let entity = receipt.entity_id.as_bytes();
    let device = receipt.receipt_data.device_id.as_bytes();
    ensure!(
        entity.len() >= 32 && device.len() >= 32,
        "Entitlement receipt has an invalid identity"
    );
    let mut result = HashMap::new();
    for entitlement in receipt.entitlement_receipts {
        if entitlement.amount == 0 {
            continue;
        }
        if let Some(expiration) = entitlement
            .expiration_date
            .filter(|d| !d.starts_with("0001-"))
            && OffsetDateTime::parse(&expiration, &Rfc3339).context("Invalid entitlement expiry")?
                <= now
        {
            continue;
        }
        if let Some(start) = entitlement.start_date.filter(|d| !d.starts_with("0001-"))
            && OffsetDateTime::parse(&start, &Rfc3339).context("Invalid entitlement start date")?
                > now
        {
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
    let auth = services.authorize().await?;
    let response = http
        .get(INVENTORY)
        .header("Authorization", auth.header_value()?)
        .send()
        .await
        .map_err(|e| e.without_url())?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        services.reject(&auth);
    }
    let inventory = json_response(response).await?;
    receipt_keys(
        inventory["result"]["receipt"]
            .as_str()
            .context("Inventory has no receipt")?,
        OffsetDateTime::now_utc(),
    )
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
        .map_err(|e| e.without_url())?;
    json_response(response)
        .await
        .with_context(|| format!("Marketplace catalog {method} failed"))
}
fn version(value: &str) -> Result<[u32; 3]> {
    let parts = value
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<Vec<u32>, _>>()?;
    parts
        .try_into()
        .map_err(|_| anyhow::anyhow!("Catalog has an invalid pack version"))
}
pub fn parse_addon(value: &Value, owned: &HashSet<Uuid>) -> Result<Option<Addon>> {
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
            version: version(
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
        .or(value["Title"]["NEUTRAL"].as_str())
        .or(value["Title"]["neutral"].as_str())
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
    let ids: Vec<_> = owned.iter().collect();
    let mut items = BTreeMap::new();
    for chunk in ids.chunks(6) {
        let filter = chunk
            .iter()
            .map(|id| format!("tags/any(t:t eq '{id}')"))
            .collect::<Vec<_>>()
            .join(" or ");
        let mut continuation: Option<String> = None;
        for page in 0..20 {
            let mut body = json!({"Top":50,"Filter":filter});
            if let Some(token) = &continuation {
                body["ContinuationToken"] = json!(token);
            }
            let response = catalog(services, http, "Search", body, true).await?;
            for item in response["data"]["Items"]
                .as_array()
                .context("Catalog search has no items")?
            {
                if let Some(addon) = parse_addon(item, &owned)?.filter(|a| a.owned) {
                    items.insert(addon.id, addon);
                }
            }
            continuation = response["data"]["ContinuationToken"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            if continuation.is_none() {
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
    let owned = inventory(services, http).await?.into_keys().collect();
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
    let response = http.post("https://entitlements.mktpl.minecraft-services.net/api/v1.0/transaction/virtual").header("Authorization",auth.header_value()?).json(&json!({"CustomTags":{"CorrelationId":Uuid::new_v4()},"OfferId":id,"StoreId":"","VirtualCurrency":{"Amount":"0","Type":"Minecoin"}})).send().await.map_err(|e|e.without_url())?;
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
    let current = parse_addon(item, &keys.keys().copied().collect())?
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
    let content = item["Contents"]
        .as_array()
        .context("Catalog has no downloadable content")?
        .iter()
        .find(|c| c["Type"].as_str() == Some("resourcebinary"))
        .context("No supported add-on download is available")?;
    let url = validate_asset_url(
        content["Url"]
            .as_str()
            .context("Catalog download has no URL")?,
    )?;
    let response = http
        .get(url)
        .header("User-Agent", "libhttpclient/1.0.0.0")
        .send()
        .await
        .map_err(|e| e.without_url())?;
    let bytes = bounded_response(response, MAX_ARCHIVE)
        .await
        .context("Marketplace asset download failed")?;
    let expected = addon.packs.iter().map(|p| (p.uuid, p.version)).collect();
    if bytes.is_empty() {
        bail!("Marketplace returned an empty add-on download");
    }
    decode_bundle(&bytes, &keys, &expected)
}
