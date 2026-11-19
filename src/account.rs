use anyhow::{Context, Result, bail};
use bedrock_services::{
    minecraft::{Config as MinecraftConfig, Minecraft},
    oauth::{Credentials, MicrosoftAuth, Poll},
    xbox::{Checkpoint, Config, MAX_CHECKPOINT_BYTES},
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    time::{Duration, SystemTime},
};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub gamertag: String,
    pub xuid: String,
}

pub struct Accounts {
    entry: keyring::Entry,
    pub services: Option<Minecraft>,
    device: Uuid,
}

impl Accounts {
    pub fn new(device: Uuid, root: &Path) -> Result<Self> {
        use sha2::{Digest, Sha256};
        let identity = format!(
            "account-{:x}",
            Sha256::digest(root.to_string_lossy().as_bytes())
        );
        Ok(Self {
            entry: keyring::Entry::new("org.stackanvil.BedrockFoundry", &identity)?,
            services: None,
            device,
        })
    }
    fn services(&self, account: bedrock_services::xbox::Account) -> Result<Minecraft> {
        Ok(Minecraft::new(
            account,
            MinecraftConfig::desktop("1.26.60", 2225, self.device),
        )?)
    }
    pub fn restore(&mut self) -> Result<bool> {
        let bytes = match self.entry.get_secret() {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(keyring::Error::NoEntry) => return Ok(false),
            Err(e) => {
                return Err(e).context(
                    "Cannot open the OS credential store. Unlock your keychain and retry.",
                );
            }
        };
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            bail!("Saved account exceeds the checkpoint size limit");
        }
        let account = MicrosoftAuth::new(Config::default())?
            .restore_account(Checkpoint::from_bytes(bytes)?)?;
        self.services = Some(self.services(account)?);
        Ok(true)
    }
    pub fn persist(&self) -> Result<()> {
        if let Some(services) = &self.services {
            self.entry.set_secret(services.checkpoint()?.as_bytes()).context("Cannot save the account in the OS credential store. Unlock your keychain and retry.")?;
        }
        Ok(())
    }
    pub async fn info(&mut self) -> Result<AccountInfo> {
        let services = self
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let result = services.xbox_authorization().await.map(|auth| AccountInfo {
            gamertag: auth.gamertag().into(),
            xuid: auth.xuid().into(),
        });
        self.persist()?;
        Ok(result?)
    }
    pub async fn sign_in(
        &mut self,
        display: impl Fn(String, String),
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<AccountInfo> {
        let mut auth = MicrosoftAuth::new(Config::default())?;
        let mut attempt = auth.start(SystemTime::now()).await?;
        display(
            attempt.verification_uri().to_string(),
            attempt.user_code().to_owned(),
        );
        let credentials = loop {
            while SystemTime::now() < attempt.next_poll() {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    bail!("Sign-in canceled");
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                bail!("Sign-in canceled");
            }
            match auth.poll(&mut attempt, SystemTime::now()).await? {
                Poll::Pending => {}
                Poll::Authorized(credentials) => break credentials,
            }
        };
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("Sign-in canceled");
        }
        self.services = Some(self.services(auth.into_account(credentials)?)?);
        // Save before the next acquisition, including when Xbox profile setup is incomplete.
        self.persist()?;
        self.info().await
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "Legacy import is a synchronous backend operation"
    )]
    pub fn import_legacy(&mut self, path: &Path) -> Result<()> {
        let bytes = std::fs::read(path)?;
        if bytes.len() > 1024 * 1024 {
            bail!("Account import is too large");
        }
        let v: serde_json::Value = serde_json::from_slice(&bytes)?;
        let client = v["msaApplicationConfig"]["clientId"]
            .as_str()
            .context("Missing OAuth client ID")?;
        if client != Config::default().client_id {
            bail!("This account uses a different OAuth application. Use Connect account instead.");
        }
        let token = &v["msaToken"];
        let credentials = Credentials::from_tokens(
            client.to_owned(),
            token["accessToken"].as_str().unwrap_or_default().into(),
            token["refreshToken"]
                .as_str()
                .context("Missing refresh credential")?
                .into(),
            SystemTime::UNIX_EPOCH
                + Duration::from_millis(token["expireTimeMs"].as_u64().unwrap_or_default()),
        )?;
        let auth = MicrosoftAuth::new(Config::default())?;
        self.services = Some(self.services(auth.into_account(credentials)?)?);
        self.persist()
    }
    pub fn sign_out(&mut self) -> Result<()> {
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        }
        self.services = None;
        Ok(())
    }
}
