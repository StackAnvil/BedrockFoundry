use super::{Backend, Context, Result, Uuid, ensure, marketplace};
use marketplace::{CatalogQuery, PurchaseAttempt, PurchaseQuote, TransactionFailure};
use sha2::{Digest, Sha256};

impl Backend {
    async fn purchase_account(&mut self) -> Result<String> {
        let info = self.accounts.info().await?;
        Ok(format!("{:x}", Sha256::digest(info.xuid.as_bytes())))
    }
    pub fn clear_marketplace(&mut self) {
        self.catalog = None;
        self.wallet = None;
        self.purchase_quote = None;
        self.library.clear();
    }
    pub async fn search_catalog(&mut self, query: CatalogQuery) -> Result<()> {
        let services = self
            .accounts
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let result = marketplace::browse(services, &self.http, query).await;
        self.accounts.persist()?;
        self.catalog = Some(result?);
        self.purchase_quote = None;
        Ok(())
    }
    pub async fn refresh_balance(&mut self) -> Result<u64> {
        self.wallet = None;
        let services = self
            .accounts
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let result = marketplace::minecoin_balance(services, &self.http).await;
        self.accounts.persist()?;
        let amount = result?;
        self.wallet = Some(amount);
        Ok(amount)
    }
    pub async fn quote_purchase(&mut self, id: Uuid) -> Result<PurchaseQuote> {
        self.purchase_quote = None;
        let account = self.purchase_account().await?;
        let services = self
            .accounts
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let result = marketplace::published_item(services, &self.http, id).await;
        self.accounts.persist()?;
        let item = result?;
        if item.owned {
            PurchaseAttempt::clear(&self.store, &account, id)?;
            anyhow::bail!(
                "This account already owns this add-on. Refresh your library to download it."
            );
        }
        let balance = self.refresh_balance().await?;
        let quote = PurchaseQuote::new(
            item,
            balance,
            account,
            time::OffsetDateTime::now_utc().unix_timestamp(),
        )?;
        ensure!(
            PurchaseAttempt::read(&self.store, &quote)?.is_none(),
            "A previous purchase is awaiting ownership confirmation. Refresh your library and check the purchase in Minecraft before trying again."
        );
        self.purchase_quote = Some(quote.clone());
        Ok(quote)
    }
    /// Consumes one reviewed quote. Accepted purchases remain accepted when a
    /// later entitlement or credential refresh fails.
    pub async fn buy(&mut self, quote_id: Uuid) -> Result<bool> {
        let quote = self
            .purchase_quote
            .take()
            .context("Review the purchase before confirming it")?;
        ensure!(
            quote.id == quote_id,
            "This purchase confirmation is no longer current"
        );
        let account = self.purchase_account().await?;
        let services = self
            .accounts
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let current = marketplace::published_item(services, &self.http, quote.item.id).await?;
        let balance = self.refresh_balance().await?;
        quote.validate(
            &current,
            balance,
            &account,
            time::OffsetDateTime::now_utc().unix_timestamp(),
        )?;
        let mut attempt = PurchaseAttempt::reserve(&self.store, &quote)?;
        let services = self
            .accounts
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let result =
            marketplace::submit_purchase(services, &self.http, &quote, attempt.correlation).await;
        // An authorization checkpoint failure must not turn an accepted charge
        // into an apparent rejection. The durable journal still blocks repeats.
        let _ = self.accounts.persist();
        if let Err(error) = result {
            if matches!(
                error,
                TransactionFailure::NotSubmitted | TransactionFailure::Rejected(_)
            ) {
                PurchaseAttempt::clear(&self.store, &account, quote.item.id)?;
            }
            return Err(error.into());
        }
        attempt.accepted = true;
        let _ = attempt.save(&self.store, &quote);
        let ready = self.refresh_library().await.is_ok()
            && self.library.iter().any(|addon| addon.id == quote.item.id);
        let _ = self.refresh_balance().await;
        Ok(ready)
    }
    pub(super) async fn reconcile_owned_purchases(&mut self) -> Result<()> {
        let account = self.purchase_account().await?;
        for addon in &self.library {
            PurchaseAttempt::clear(&self.store, &account, addon.id)?;
        }
        if let Some(page) = &mut self.catalog {
            for item in &mut page.items {
                if item.addon.is_some() {
                    item.owned = self.library.iter().any(|addon| addon.id == item.id);
                    if let Some(addon) = &mut item.addon {
                        addon.owned = item.owned;
                    }
                }
            }
        }
        Ok(())
    }
}
