use crate::{
    account::{AccountInfo, Accounts},
    archive::{Manifest, write_pack},
    marketplace::{self, Addon},
    server::{self, Processes, ServerConfig},
    storage::{Store, atomic_json},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};
use uuid::Uuid;

mod commerce;

#[derive(Serialize, Deserialize)]
struct Identity {
    device: Uuid,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct CachedAddon {
    pub addon: Addon,
    pub manifests: Vec<Manifest>,
}
pub struct Backend {
    pub store: Store,
    pub accounts: Accounts,
    pub http: reqwest::Client,
    pub processes: Processes,
    pub library: Vec<Addon>,
    pub available: HashMap<Uuid, CachedAddon>,
    pub catalog: Option<marketplace::CatalogPage>,
    pub wallet: Option<u64>,
    pub purchase_quote: Option<marketplace::PurchaseQuote>,
}
impl Backend {
    #[expect(
        clippy::disallowed_methods,
        reason = "The CLI and desktop worker open the backend synchronously"
    )]
    pub fn open(root: Option<PathBuf>) -> Result<Self> {
        let store = Store::open(root)?;
        server::recover_installations(&store.root)?;
        let identity: Identity = store.read("device.json")?.unwrap_or_else(|| Identity {
            device: Uuid::new_v4(),
        });
        store.save("device.json", &identity)?;
        let accounts = Accounts::new(identity.device, &store.root)?;
        let mut available = HashMap::new();
        for entry in std::fs::read_dir(store.root.join("addons"))? {
            let entry = entry?;
            let file = entry.path().join("addon.json");
            if file.is_file() {
                let cached: CachedAddon = serde_json::from_slice(&std::fs::read(file)?)?;
                ensure!(
                    entry.file_name().to_string_lossy() == cached.addon.id.to_string(),
                    "Cached add-on has a different identity"
                );
                available.insert(cached.addon.id, cached);
            }
        }
        Ok(Self {
            store,
            accounts,
            http: marketplace::client()?,
            processes: Processes::default(),
            library: vec![],
            available,
            catalog: None,
            wallet: None,
            purchase_quote: None,
        })
    }
    pub fn servers(&self) -> Result<Vec<ServerConfig>> {
        server::list(&self.store.root)
    }
    pub fn server(&self, id: Uuid) -> Result<ServerConfig> {
        self.servers()?
            .into_iter()
            .find(|s| s.id == id)
            .context("Server instance does not exist")
    }
    pub fn create(&self, name: String, port: u16) -> Result<ServerConfig> {
        let config = ServerConfig::new(name, port)?;
        let servers = self.servers()?;
        ensure!(
            !servers
                .iter()
                .any(|s| s.port == port || s.port == port + 1 || s.port + 1 == port),
            "Another server already uses this port or its IPv6 neighbor"
        );
        server::save(&self.store.root, &config)?;
        Ok(config)
    }
    pub async fn restore(&mut self) -> Result<Option<AccountInfo>> {
        if self.accounts.restore()? {
            Ok(Some(self.accounts.info().await?))
        } else {
            Ok(None)
        }
    }
    pub async fn refresh_library(&mut self) -> Result<Vec<Addon>> {
        let services = self
            .accounts
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let result = marketplace::list(services, &self.http).await;
        self.accounts.persist()?;
        self.library = result?;
        self.reconcile_owned_purchases().await?;
        Ok(self.library.clone())
    }
    pub async fn sample(&mut self) -> Result<Addon> {
        let services = self
            .accounts
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let result = marketplace::free_sample(services, &self.http).await;
        self.accounts.persist()?;
        result
    }
    pub async fn claim(&mut self, id: Uuid) -> Result<()> {
        let services = self
            .accounts
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let result = marketplace::claim_free(services, &self.http, id).await;
        self.accounts.persist()?;
        result
    }
    pub async fn download(&mut self, id: Uuid) -> Result<()> {
        if !self.library.iter().any(|addon| addon.id == id) {
            let services = self
                .accounts
                .services
                .as_mut()
                .context("Connect your Microsoft account first")?;
            let item = marketplace::published_item(services, &self.http, id).await?;
            ensure!(item.owned, "This account does not own this add-on");
            self.library.push(
                item.addon
                    .context("This listing is not a standalone add-on")?,
            );
        }
        let addon = self
            .library
            .iter()
            .find(|a| a.id == id)
            .context("Refresh your library before downloading this add-on")?
            .clone();
        let services = self
            .accounts
            .services
            .as_mut()
            .context("Connect your Microsoft account first")?;
        let result = marketplace::download(services, &self.http, &addon).await;
        self.accounts.persist()?;
        let packs = result?;
        let parent = self.store.root.join("addons");
        let cached = tokio::task::spawn_blocking(move || {
            let destination = parent.join(id.to_string());
            ensure!(!destination.exists(), "This add-on is already downloaded");
            let staging = tempfile::tempdir_in(&parent)?;
            for pack in &packs {
                write_pack(
                    pack,
                    &staging.path().join(pack.manifest.header.uuid.to_string()),
                )?;
            }
            let cached = CachedAddon {
                addon,
                manifests: packs.iter().map(|p| p.manifest.clone()).collect(),
            };
            atomic_json(&staging.path().join("addon.json"), &cached)?;
            std::fs::rename(staging.path(), &destination)?;
            Ok::<_, anyhow::Error>(cached)
        })
        .await??;
        self.available.insert(id, cached);
        Ok(())
    }
    pub fn install(&mut self, server: Uuid, addon: Uuid) -> Result<()> {
        #[expect(
            clippy::disallowed_methods,
            reason = "Synchronous add-on installation runs on the desktop worker or CLI thread"
        )]
        fn collect(
            base: &std::path::Path,
            dir: &std::path::Path,
            files: &mut std::collections::BTreeMap<String, Vec<u8>>,
        ) -> Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let meta = std::fs::symlink_metadata(entry.path())?;
                ensure!(
                    !meta.file_type().is_symlink(),
                    "Cached pack contains a symbolic link"
                );
                if meta.is_dir() {
                    collect(base, &entry.path(), files)?;
                } else if meta.is_file() {
                    files.insert(
                        entry
                            .path()
                            .strip_prefix(base)?
                            .to_string_lossy()
                            .replace('\\', "/"),
                        std::fs::read(entry.path())?,
                    );
                }
            }
            Ok(())
        }

        self.processes.require_stopped(server)?;
        let cached = self
            .available
            .get(&addon)
            .context("Download this add-on first")?;
        let directory = self.store.root.join("addons").join(addon.to_string());
        let mut packs = Vec::new();
        for manifest in &cached.manifests {
            let base = directory.join(manifest.header.uuid.to_string());
            let mut files = std::collections::BTreeMap::new();
            collect(&base, &base, &mut files)?;
            packs.push(crate::archive::DecodedPack {
                manifest: manifest.clone(),
                files,
            });
        }
        let mut config = self.server(server)?;
        server::install_addon(&self.store.root, &mut config, &cached.addon, &packs)
    }
    pub async fn provision(&mut self, id: Uuid) -> Result<()> {
        self.processes.require_stopped(id)?;
        let mut config = self.server(id)?;
        server::download_server(&self.store.root, &mut config, &self.http).await
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "Synchronous BDS import runs on the desktop worker"
    )]
    pub fn import_server(&mut self, id: Uuid, path: PathBuf) -> Result<()> {
        self.processes.require_stopped(id)?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("Choose an official bedrock-server-VERSION.zip file")?;
        let version = filename
            .strip_prefix("bedrock-server-")
            .and_then(|n| n.strip_suffix(".zip"))
            .context("Choose an official bedrock-server-VERSION.zip file")?
            .to_string();
        ensure!(
            version.split('.').count() == 4
                && version
                    .split('.')
                    .all(|part| !part.is_empty() && part.parse::<u32>().is_ok()),
            "The BDS filename has an invalid version"
        );
        ensure!(
            std::fs::metadata(&path)?.len() <= crate::archive::MAX_ARCHIVE as u64,
            "BDS ZIP exceeds the download size limit"
        );
        let bytes = std::fs::read(path)?;
        let mut config = self.server(id)?;
        server::import_distribution(&self.store.root, &mut config, &bytes, version)
    }
    pub fn update(&mut self, config: &ServerConfig) -> Result<()> {
        config.validate()?;
        self.processes.require_stopped(config.id)?;
        self.server(config.id)?;
        ensure!(
            !self.servers()?.iter().any(|other| other.id != config.id
                && (other.port == config.port
                    || other.port == config.port + 1
                    || other.port + 1 == config.port)),
            "Another server already uses this port or its UDP neighbor"
        );
        server::save(&self.store.root, config)
    }
    pub fn backup(&mut self, id: Uuid) -> Result<PathBuf> {
        self.processes.require_stopped(id)?;
        server::backup(&self.store.root, &self.server(id)?)
    }
    pub fn start(&mut self, id: Uuid) -> Result<()> {
        let config = self.server(id)?;
        self.processes.start(&self.store.root, &config)
    }
    pub fn folder(&self, id: Uuid) -> PathBuf {
        server::home(&self.store.root, id)
    }
}
