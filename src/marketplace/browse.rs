use super::*;

pub const PAGE_SIZE: u64 = 24;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CatalogQuery {
    pub search: String,
    pub addons_only: bool,
    pub skip: u64,
}
impl CatalogQuery {
    pub fn body(&self) -> Result<Value> {
        ensure!(
            self.search.len() <= 200 && !self.search.chars().any(char::is_control),
            "Search must contain at most 200 bytes and no control characters"
        );
        ensure!(self.skip <= 10_000_000, "Catalog page is out of range");
        // Customer offers are bundles. Raw catalogItem records include internal
        // pack payloads, entitlement shells, and real-money currency SKUs.
        let mut filter = "type eq 'bundle' and not tags/any(t:t eq 'hidden_offer')".to_owned();
        if self.addons_only {
            filter.push_str(" and tags/any(t:t eq 'addon')");
        }
        // Default relevance ties can move between offset pages. Use a total
        // order so an unchanged catalog cannot repeat or skip tied results.
        let mut body = json!({"Top": PAGE_SIZE, "Skip": self.skip, "Filter": filter, "OrderBy":"creationDate desc, id asc"});
        if !self.search.trim().is_empty() {
            body["Search"] = json!(self.search.trim());
        }
        Ok(body)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Ownership {
    pub offers: HashSet<Uuid>,
    pub packs: HashSet<Uuid>,
}
pub fn receipt_ownership(encoded: &str, now: OffsetDateTime) -> Result<Ownership> {
    let receipt = decode_receipt(encoded)?;
    let mut owned = Ownership::default();
    for entitlement in receipt.entitlement_receipts {
        if active(&entitlement, now)? {
            owned.packs.insert(entitlement.pack_id);
            owned.offers.extend(entitlement.offer_ids);
        }
    }
    Ok(owned)
}
pub(super) async fn ownership(services: &mut Minecraft, http: &Client) -> Result<Ownership> {
    receipt_ownership(
        &inventory_receipt(services, http).await?,
        OffsetDateTime::now_utc(),
    )
}
fn localized(value: &Value) -> Option<&str> {
    ["en-US", "NEUTRAL", "neutral"]
        .into_iter()
        .filter_map(|locale| value[locale].as_str())
        .find(|s| !s.trim().is_empty())
        .or_else(|| {
            value
                .as_object()?
                .values()
                .filter_map(Value::as_str)
                .find(|s| !s.trim().is_empty())
        })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: Uuid,
    pub title: String,
    pub creator: String,
    pub description: String,
    pub thumbnail: Option<String>,
    pub content_type: String,
    pub kind: String,
    pub price: Option<u64>,
    pub purchase_price: Option<u64>,
    pub owned: bool,
    pub purchasable: bool,
    pub addon: Option<Addon>,
}
impl CatalogItem {
    pub fn parse(value: &Value, owned: &Ownership) -> Result<Self> {
        let id = value["Id"]
            .as_str()
            .context("Catalog listing has no ID")?
            .parse()?;
        let tags: HashSet<_> = value["Tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let content_type = value["ContentType"].as_str().unwrap_or("Other").to_owned();
        let kind = if tags.contains("addon") {
            "Add-on"
        } else if tags.contains("worldtemplate") {
            "World"
        } else if tags.contains("skinpack") || content_type == "skinpack" {
            "Skin pack"
        } else if tags.contains("resourcepack") || tags.contains("texturepack") {
            "Texture pack"
        } else if content_type.contains("Persona") {
            "Character creator"
        } else {
            &content_type
        }
        .to_owned();
        let mut addon = parse_addon(value, &owned.packs)?;
        let pack_ids = value["DisplayProperties"]["packIdentity"]
            .as_array()
            .and_then(|packs| {
                packs
                    .iter()
                    .map(|pack| pack["uuid"].as_str()?.parse::<Uuid>().ok())
                    .collect::<Option<Vec<_>>>()
            });
        let is_owned = owned.offers.contains(&id)
            || pack_ids.as_ref().is_some_and(|packs| {
                !packs.is_empty() && packs.iter().all(|pack| owned.packs.contains(pack))
            });
        if let Some(addon) = &mut addon {
            addon.owned = is_owned;
        }
        Ok(Self {
            id,
            title: localized(&value["Title"])
                .unwrap_or("Unnamed listing")
                .to_owned(),
            creator: value["DisplayProperties"]["creatorName"]
                .as_str()
                .unwrap_or("Unknown creator")
                .to_owned(),
            description: localized(&value["Description"])
                .unwrap_or_default()
                .chars()
                .take(4000)
                .collect(),
            thumbnail: value["Images"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|image| {
                    image["Type"].as_str() == Some("Thumbnail")
                        || image["Tag"].as_str() == Some("Thumbnail")
                })
                .and_then(|image| image["Url"].as_str())
                .filter(|url| validate_asset_url(url).is_ok())
                .map(str::to_owned),
            content_type,
            kind,
            price: value["DisplayProperties"]["price"].as_u64(),
            owned: is_owned,
            purchase_price: super::purchase::verified_price(value),
            purchasable: value["DisplayProperties"]["purchasable"].as_bool() == Some(true),
            addon,
        })
    }
    pub fn price_label(&self) -> String {
        if self.owned {
            "Owned".into()
        } else {
            match self.price {
                Some(0) => "Free".into(),
                Some(amount) => format!("{amount} Minecoins"),
                None => "Price unavailable".into(),
            }
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CatalogPage {
    pub query: CatalogQuery,
    pub total: u64,
    pub items: Vec<CatalogItem>,
    pub next_skip: Option<u64>,
}
impl CatalogPage {
    pub fn parse(value: &Value, query: CatalogQuery, owned: &Ownership) -> Result<Self> {
        let data = &value["data"];
        let total = data["Count"]
            .as_u64()
            .context("Catalog response has no total count")?;
        let records = data["Items"]
            .as_array()
            .context("Catalog response has no items")?;
        ensure!(
            records.len() <= PAGE_SIZE as usize,
            "Catalog returned too many listings"
        );
        let next = query
            .skip
            .checked_add(records.len() as u64)
            .context("Catalog offset overflow")?;
        let items = records
            .iter()
            .map(|record| CatalogItem::parse(record, owned))
            .collect::<Result<_>>()?;
        Ok(Self {
            next_skip: (!records.is_empty() && next < total).then_some(next),
            query,
            total,
            items,
        })
    }
}

pub async fn browse(
    services: &mut Minecraft,
    http: &Client,
    query: CatalogQuery,
) -> Result<CatalogPage> {
    let body = query.body()?;
    let owned = ownership(services, http).await?;
    let response = catalog(services, http, "Search", body, true).await?;
    CatalogPage::parse(&response, query, &owned)
}
pub(crate) async fn published(
    services: &mut Minecraft,
    http: &Client,
    id: Uuid,
) -> Result<CatalogItem> {
    let owned = ownership(services, http).await?;
    let value = catalog(
        services,
        http,
        "GetPublishedItem",
        json!({"ItemId":id}),
        false,
    )
    .await?;
    let item = CatalogItem::parse(&value["data"]["Item"], &owned)?;
    ensure!(item.id == id, "Marketplace returned a different listing");
    Ok(item)
}
