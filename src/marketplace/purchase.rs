use super::{
    CatalogItem, Client, Context, Deserialize, Minecraft, Result, Serialize, Uuid, Value,
    bounded_response, ensure, json, json_response,
};
use crate::storage::Store;
use reqwest::header::HeaderValue;
use std::{fmt, path::PathBuf};

const BALANCE: &str =
    "https://entitlements.mktpl.minecraft-services.net/api/v1.0/currencies/virtual/balances";
const TRANSACTION: &str =
    "https://entitlements.mktpl.minecraft-services.net/api/v1.0/transaction/virtual";
const MINECOIN: &str = "ecd19d3c-7635-402c-a185-eb11cb6c6946";

pub fn parse_balance(value: &Value) -> Result<u64> {
    let rows = value["result"]["virtualCurrencyBalances"]
        .as_array()
        .context("Marketplace returned no currency balances")?;
    let amounts: Vec<_> = rows
        .iter()
        .filter(|row| row["type"].as_str() == Some("Minecoin"))
        .collect();
    ensure!(
        amounts.len() == 1,
        "Marketplace returned an ambiguous Minecoin balance"
    );
    amounts[0]["amount"]
        .as_u64()
        .context("Marketplace returned an invalid Minecoin balance")
}
pub async fn minecoin_balance(services: &mut Minecraft, http: &Client) -> Result<u64> {
    let auth = services.authorize().await?;
    let response = http
        .post(BALANCE)
        .header("Authorization", auth.header_value()?)
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        services.reject(&auth);
    }
    parse_balance(&json_response(response).await?)
}

pub(super) fn verified_price(value: &Value) -> Option<u64> {
    let display = value["DisplayProperties"]["price"].as_u64()?;
    // A different store, currency, discount, or composite price needs a new quote.
    for field in ["Price", "PriceOptions"] {
        let mut amounts = Vec::new();
        for price in value[field]["Prices"].as_array()? {
            let parts = price["Amounts"].as_array()?;
            if parts.len() == 1 && parts[0]["CurrencyId"].as_str() == Some(MINECOIN) {
                amounts.push(parts[0]["Amount"].as_u64()?);
            }
        }
        if amounts.len() != 1 || amounts[0] != display {
            return None;
        }
    }
    Some(display)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PurchaseQuote {
    pub id: Uuid,
    pub item: CatalogItem,
    pub amount: u64,
    pub balance: u64,
    pub expires_at: i64,
    pub(crate) account: String,
}
impl PurchaseQuote {
    pub fn new(item: CatalogItem, balance: u64, account: String, now: i64) -> Result<Self> {
        ensure!(
            account.len() == 64 && account.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid purchase account"
        );
        ensure!(!item.owned, "This account already owns this add-on");
        ensure!(
            item.addon.is_some(),
            "Only standalone add-ons can be purchased here"
        );
        ensure!(
            item.purchasable,
            "This add-on is not currently available for purchase"
        );
        let amount = item
            .purchase_price
            .context("This offer has no verified Minecoin price. Buy it in Minecraft instead.")?;
        Ok(Self {
            id: Uuid::new_v4(),
            item,
            amount,
            balance,
            account,
            expires_at: now + 300,
        })
    }
    pub fn validate(
        &self,
        current: &CatalogItem,
        balance: u64,
        account: &str,
        now: i64,
    ) -> Result<()> {
        ensure!(
            self.account == account,
            "The connected account changed. Review the purchase again."
        );
        ensure!(
            now < self.expires_at,
            "The purchase quote expired. Review the purchase again."
        );
        ensure!(
            current.id == self.item.id && !current.owned,
            "This add-on is already owned or the listing changed"
        );
        ensure!(
            current.purchasable
                && current.purchase_price == Some(self.amount)
                && current.price == Some(self.amount),
            "The price or availability changed. Review the purchase again."
        );
        ensure!(
            current.addon.as_ref().map(|a| &a.packs) == self.item.addon.as_ref().map(|a| &a.packs)
                && current.addon.is_some(),
            "The add-on packs changed. Review the purchase again."
        );
        ensure!(
            balance >= self.amount,
            "Not enough Minecoins. Add Minecoins in Minecraft, then refresh your balance."
        );
        Ok(())
    }
    #[must_use]
    pub fn transaction_body(&self, correlation: Uuid) -> Value {
        json!({"CustomTags":{"CorrelationId":correlation},"OfferId":self.item.id,"StoreId":"","VirtualCurrency":{"Amount":self.amount.to_string(),"Type":"Minecoin"}})
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PurchaseAttempt {
    pub offer: Uuid,
    pub amount: u64,
    pub correlation: Uuid,
    pub accepted: bool,
}
impl PurchaseAttempt {
    fn name(account: &str, offer: Uuid) -> String {
        format!("purchases/{account}-{offer}.json")
    }
    pub fn read(store: &Store, quote: &PurchaseQuote) -> Result<Option<Self>> {
        store.read(&Self::name(&quote.account, quote.item.id))
    }
    pub fn reserve(store: &Store, quote: &PurchaseQuote) -> Result<Self> {
        ensure!(
            Self::read(store, quote)?.is_none(),
            "A previous purchase is awaiting ownership confirmation. Refresh your library and check the purchase in Minecraft before trying again."
        );
        let attempt = Self {
            offer: quote.item.id,
            amount: quote.amount,
            correlation: Uuid::new_v4(),
            accepted: false,
        };
        attempt.save(store, quote)?;
        Ok(attempt)
    }
    pub fn save(&self, store: &Store, quote: &PurchaseQuote) -> Result<()> {
        store.save(&Self::name(&quote.account, quote.item.id), self)
    }
    pub fn clear(store: &Store, account: &str, offer: Uuid) -> Result<()> {
        let path: PathBuf = store.root.join(Self::name(account, offer));
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TransactionFailure {
    NotSubmitted,
    Rejected(u16),
    Unknown,
}
impl fmt::Display for TransactionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSubmitted => write!(
                f,
                "Could not authorize the purchase. No purchase request was sent."
            ),
            Self::Rejected(status) => write!(
                f,
                "Marketplace rejected the purchase (HTTP {status}). Refresh the listing and balance."
            ),
            Self::Unknown => write!(
                f,
                "Purchase outcome is unknown. Do not repeat it. Refresh your library and check the purchase in Minecraft."
            ),
        }
    }
}
impl std::error::Error for TransactionFailure {}

#[expect(
    clippy::map_err_ignore,
    reason = "Transport and response errors can contain signed URLs or sensitive receipt data"
)]
pub(super) async fn transaction(
    http: &Client,
    url: &str,
    auth: HeaderValue,
    quote: &PurchaseQuote,
    correlation: Uuid,
) -> std::result::Result<(), TransactionFailure> {
    let response = http
        .post(url)
        .header("Authorization", auth)
        .json(&quote.transaction_body(correlation))
        .send()
        .await
        .map_err(|_| TransactionFailure::Unknown)?;
    let status = response.status().as_u16();
    if matches!(status, 400 | 401 | 402 | 403 | 404 | 422) {
        return Err(TransactionFailure::Rejected(status));
    }
    let bytes = bounded_response(response, 1024 * 1024)
        .await
        .map_err(|_| TransactionFailure::Unknown)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| TransactionFailure::Unknown)?;
    if value["result"]["offerId"].as_str() != Some(&quote.item.id.to_string())
        || !value["result"]["transactionIds"]
            .as_array()
            .is_some_and(|ids| {
                !ids.is_empty()
                    && ids
                        .iter()
                        .all(|id| id.as_str().is_some_and(|s| !s.is_empty()))
            })
    {
        return Err(TransactionFailure::Unknown);
    }
    Ok(())
}
#[expect(
    clippy::map_err_ignore,
    reason = "Authorization errors can expose credential details and callers only need to know that no purchase was submitted"
)]
pub(crate) async fn submit_purchase(
    services: &mut Minecraft,
    http: &Client,
    quote: &PurchaseQuote,
    correlation: Uuid,
) -> std::result::Result<(), TransactionFailure> {
    let auth = services
        .authorize()
        .await
        .map_err(|_| TransactionFailure::NotSubmitted)?;
    let header = auth
        .header_value()
        .map_err(|_| TransactionFailure::NotSubmitted)?;
    let result = transaction(http, TRANSACTION, header, quote, correlation).await;
    if result == Err(TransactionFailure::Rejected(401)) {
        services.reject(&auth);
    }
    result
}
