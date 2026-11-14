use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use fs2::FileExt;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub struct Store {
    pub root: PathBuf,
    _lease: File,
}

impl Store {
    pub fn open(root: Option<PathBuf>) -> Result<Self> {
        let root = root
            .or_else(|| std::env::var_os("FOUNDRY_HOME").map(PathBuf::from))
            .or_else(|| {
                ProjectDirs::from("org", "StackAnvil", "BedrockFoundry")
                    .map(|p| p.data_local_dir().to_path_buf())
            })
            .context("Cannot find the application data directory")?;
        private_dir(&root)?;
        let root = fs::canonicalize(root)?;
        let lease = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("app.lock"))?;
        lease.try_lock_exclusive().context("Bedrock Foundry is already using this data directory. Close its other window or CLI process.")?;
        for directory in ["servers", "addons", "backups", "downloads"] {
            private_dir(&root.join(directory))?;
        }
        Ok(Self {
            root,
            _lease: lease,
        })
    }
    pub fn read<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        let file = self.root.join(name);
        if !file.exists() {
            return Ok(None);
        }
        let data = fs::read(&file).with_context(|| format!("Cannot read {}", file.display()))?;
        Ok(Some(serde_json::from_slice(&data).with_context(|| {
            format!("Invalid configuration in {}", file.display())
        })?))
    }
    pub fn save<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        atomic_json(&self.root.join(name), value)
    }
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .context("Configuration has no parent directory")?;
    private_dir(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn safe_relative(name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || name
            .chars()
            .any(|c| c.is_control() || "\\:\"<>|?*".contains(c))
        || name.split('/').any(|part| {
            let stem = part
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        })
    {
        bail!("Archive contains an invalid path");
    }
    let path = Path::new(name);
    if path
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        bail!("Archive path escapes its destination");
    }
    Ok(path.to_path_buf())
}
