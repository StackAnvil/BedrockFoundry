use crate::{
    archive::{DecodedPack, MAX_ARCHIVE, Manifest, read_zip, write_pack},
    marketplace::{Addon, bounded_response},
    storage::{atomic_json, private_dir, safe_relative},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use zip::{ZipWriter, write::SimpleFileOptions};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    pub id: Uuid,
    pub name: String,
    pub port: u16,
    pub max_players: u16,
    pub gamemode: String,
    pub difficulty: String,
    pub public: bool,
    #[serde(default)]
    pub public_address: Option<std::net::IpAddr>,
    pub addons: Vec<InstalledAddon>,
    pub version: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledAddon {
    pub id: Uuid,
    pub name: String,
    pub packs: Vec<Manifest>,
}

impl ServerConfig {
    pub fn new(name: String, port: u16) -> Result<Self> {
        let config = Self {
            id: Uuid::new_v4(),
            name,
            port,
            max_players: 10,
            gamemode: "survival".into(),
            difficulty: "normal".into(),
            public: false,
            public_address: None,
            addons: vec![],
            version: None,
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty()
                && self.name.chars().count() <= 80
                && !self.name.chars().any(char::is_control)
                && !self.name.contains(';'),
            "Use a server name of 1 to 80 characters without control characters or semicolons"
        );
        ensure!(
            (1024..=65534).contains(&self.port),
            "Use a port from 1024 to 65534"
        );
        ensure!(
            self.max_players > 0 && self.max_players <= 100,
            "Use a player limit from 1 to 100"
        );
        ensure!(
            ["survival", "creative", "adventure"].contains(&self.gamemode.as_str()),
            "Unknown game mode"
        );
        ensure!(
            ["peaceful", "easy", "normal", "hard"].contains(&self.difficulty.as_str()),
            "Unknown difficulty"
        );
        Ok(())
    }
    pub fn properties(&self, original: &str) -> Result<String> {
        self.validate()?;
        let managed = BTreeMap::from([
            ("server-name", self.name.clone()),
            ("server-port", self.port.to_string()),
            ("server-portv6", (self.port + 1).to_string()),
            (
                "server-ip",
                if self.public { "0.0.0.0" } else { "127.0.0.1" }.into(),
            ),
            (
                "server-udp-ports",
                self.public_address
                    .filter(|_| self.public)
                    .map(|ip| match ip {
                        std::net::IpAddr::V4(ip) => {
                            format!("{ip}:{}:{}", self.port + 1, self.port + 1)
                        }
                        std::net::IpAddr::V6(ip) => {
                            format!("[{ip}]:{}:{}", self.port + 1, self.port + 1)
                        }
                    })
                    .unwrap_or_else(|| (self.port + 1).to_string()),
            ),
            ("enable-lan-visibility", "false".into()),
            ("transport", "nethernet".into()),
            ("online-mode", "true".into()),
            ("gamemode", self.gamemode.clone()),
            ("difficulty", self.difficulty.clone()),
            ("max-players", self.max_players.to_string()),
            ("level-name", "world".into()),
            ("texturepack-required", "true".into()),
            ("content-log-file-enabled", "true".into()),
        ]);
        let mut output = original
            .lines()
            .filter(|line| {
                !line.split_once('=').is_some_and(|(k, _)| {
                    managed.contains_key(k.trim()) || k.trim() == "server-bind-address"
                })
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for (key, value) in managed {
            output.push(format!("{key}={value}"));
        }
        Ok(output.join("\n") + "\n")
    }
}

pub fn home(root: &Path, id: Uuid) -> PathBuf {
    root.join("servers").join(id.to_string())
}
pub fn save(root: &Path, config: &ServerConfig) -> Result<()> {
    config.validate()?;
    let home = home(root, config.id);
    private_dir(&home)?;
    for name in ["allowlist.json", "permissions.json"] {
        if !home.join(name).exists() {
            atomic_json(&home.join(name), &Vec::<serde_json::Value>::new())?;
        }
    }
    atomic_json(&home.join("foundry.json"), config)?;
    let properties = home.join("server.properties");
    let original = fs::read_to_string(&properties).unwrap_or_default();
    let mut temp = tempfile::NamedTempFile::new_in(&home)?;
    temp.write_all(config.properties(&original)?.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(properties).map_err(|e| e.error)?;
    Ok(())
}
pub fn list(root: &Path) -> Result<Vec<ServerConfig>> {
    let mut servers = Vec::new();
    for entry in fs::read_dir(root.join("servers"))? {
        let entry = entry?;
        let file = entry.path().join("foundry.json");
        if !file.is_file() {
            continue;
        }
        let config: ServerConfig = serde_json::from_slice(&fs::read(&file)?)
            .with_context(|| format!("Invalid server configuration at {}", file.display()))?;
        ensure!(
            entry.file_name().to_string_lossy() == config.id.to_string(),
            "Server directory has a different instance identity"
        );
        config.validate()?;
        servers.push(config);
    }
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(servers)
}

#[cfg(windows)]
const BINARY: &str = "bedrock_server.exe";
#[cfg(not(windows))]
const BINARY: &str = "bedrock_server";

pub async fn download_server(
    root: &Path,
    config: &mut ServerConfig,
    http: &reqwest::Client,
) -> Result<()> {
    ensure!(
        cfg!(any(target_os = "linux", target_os = "windows")),
        "Local BDS hosting is supported on Windows and Linux"
    );
    ensure!(
        config.version.is_none(),
        "This server is already installed. Back it up before installing a different BDS version."
    );
    let metadata = bounded_response(
        http.get("https://net-secondary.web.minecraft-services.net/api/v1.0/download/links")
            .send()
            .await
            .map_err(|e| e.without_url())?,
        1024 * 1024,
    )
    .await?;
    let metadata: serde_json::Value = serde_json::from_slice(&metadata)?;
    let platform = if cfg!(windows) {
        "serverBedrockWindows"
    } else {
        "serverBedrockLinux"
    };
    let entry = metadata["result"]["links"]
        .as_array()
        .context("Download service returned no links")?
        .iter()
        .find(|e| e["downloadType"].as_str() == Some(platform))
        .context("No BDS download is available for this platform")?;
    let url = reqwest::Url::parse(
        entry["downloadUrl"]
            .as_str()
            .context("BDS download has no URL")?,
    )?;
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("www.minecraft.net")
            && url.path().starts_with("/bedrockdedicatedserver/")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443),
        "The official BDS download URL is not supported"
    );
    let filename = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .context("BDS download has no filename")?;
    let version = filename
        .strip_prefix("bedrock-server-")
        .and_then(|s| s.strip_suffix(".zip"))
        .context("BDS download has an unknown filename")?
        .to_owned();
    ensure!(
        version.split('.').count() == 4 && version.chars().all(|c| c.is_ascii_digit() || c == '.'),
        "BDS download has an invalid version"
    );
    let bytes = bounded_response(
        http.get(url).send().await.map_err(|e| e.without_url())?,
        MAX_ARCHIVE,
    )
    .await?;
    import_distribution(root, config, &bytes, version)
}

pub fn import_distribution(
    root: &Path,
    config: &mut ServerConfig,
    bytes: &[u8],
    version: String,
) -> Result<()> {
    ensure!(
        config.version.is_none(),
        "BDS is already installed in this instance"
    );
    let home = home(root, config.id);
    private_dir(&home)?;
    ensure!(
        !home.join(BINARY).exists(),
        "An existing BDS executable needs recovery. No installation files were overwritten."
    );
    let files = read_zip(bytes)?;
    ensure!(
        files.contains_key(BINARY),
        "Server archive has no executable for this platform"
    );
    let staging = tempfile::tempdir_in(&home)?;
    for (name, bytes) in files {
        // Official defaults must not replace this instance's managed configuration or world data.
        let first = name.split('/').next().unwrap_or_default();
        if [
            "worlds",
            "backups",
            "foundry.json",
            "server.properties",
            "allowlist.json",
            "permissions.json",
            "keys",
        ]
        .contains(&first)
        {
            continue;
        }
        let path = staging.path().join(safe_relative(&name)?);
        private_dir(path.parent().context("Server archive path has no parent")?)?;
        fs::write(path, bytes)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            staging.path().join(BINARY),
            fs::Permissions::from_mode(0o700),
        )?;
    }
    for entry in fs::read_dir(staging.path())? {
        let entry = entry?;
        ensure!(
            !home.join(entry.file_name()).exists(),
            "Installation would overwrite an existing server file"
        );
    }
    for entry in fs::read_dir(staging.path())? {
        let entry = entry?;
        fs::rename(entry.path(), home.join(entry.file_name()))?;
    }
    config.version = Some(version);
    save(root, config)
}

pub fn install_addon(
    root: &Path,
    config: &mut ServerConfig,
    addon: &Addon,
    packs: &[DecodedPack],
) -> Result<()> {
    ensure!(
        config.version.is_some(),
        "Install BDS before adding an add-on"
    );
    ensure!(
        !config.addons.iter().any(|a| a.id == addon.id),
        "This add-on is already installed"
    );
    let server_version = config
        .version
        .as_ref()
        .context("BDS has no version")?
        .split('.')
        .take(3)
        .map(str::parse)
        .collect::<std::result::Result<Vec<u32>, _>>()?;
    let engine: [u32; 3] = server_version
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid BDS version"))?;
    let home = home(root, config.id);
    let world = home.join("worlds/world");
    private_dir(&world)?;
    let mut ids: HashMap<Uuid, [u32; 3]> = config
        .addons
        .iter()
        .flat_map(|a| &a.packs)
        .map(|p| (p.header.uuid, p.header.version))
        .collect();
    for pack in packs {
        ensure!(
            pack.manifest
                .header
                .min_engine_version
                .is_none_or(|v| v <= engine),
            "This add-on requires a newer BDS version"
        );
        ensure!(
            ids.insert(pack.manifest.header.uuid, pack.manifest.header.version)
                .is_none(),
            "Another add-on already supplies this pack"
        );
    }
    for pack in packs {
        for dependency in &pack.manifest.dependencies {
            if let Some(id) = dependency.uuid {
                let required: [u32; 3] = serde_json::from_value(dependency.version.clone())
                    .context("Invalid pack dependency version")?;
                ensure!(
                    ids.get(&id) == Some(&required),
                    "Add-on requires a missing or different pack dependency: {id}"
                );
            }
        }
    }
    let staging = tempfile::tempdir_in(&home)?;
    for pack in packs {
        write_pack(
            pack,
            &staging.path().join(pack.manifest.header.uuid.to_string()),
        )?;
    }
    let mut destinations = Vec::new();
    for pack in packs {
        let parent = home.join(pack.manifest.kind()?);
        private_dir(&parent)?;
        let destination = parent.join(pack.manifest.header.uuid.to_string());
        ensure!(
            !destination.exists(),
            "Pack files already exist. Restore or remove the interrupted installation before retrying."
        );
        destinations.push(destination);
    }
    let previous = config.clone();
    let activation_files = ["world_behavior_packs.json", "world_resource_packs.json"]
        .into_iter()
        .map(|name| {
            let path = world.join(name);
            let bytes = if path.exists() {
                Some(fs::read(&path)?)
            } else {
                None
            };
            Ok((name.to_string(), bytes))
        })
        .collect::<Result<Vec<_>>>()?;
    let journal = InstallJournal {
        previous: previous.clone(),
        activation_files,
        created: packs
            .iter()
            .map(|p| Ok((p.manifest.kind()?.to_string(), p.manifest.header.uuid)))
            .collect::<Result<_>>()?,
    };
    atomic_json(&home.join("install-journal.json"), &journal)?;
    config.addons.push(InstalledAddon {
        id: addon.id,
        name: addon.name.clone(),
        packs: packs.iter().map(|p| p.manifest.clone()).collect(),
    });
    let operation = (|| -> Result<()> {
        for (pack, destination) in packs.iter().zip(&destinations) {
            fs::rename(
                staging.path().join(pack.manifest.header.uuid.to_string()),
                destination,
            )?;
        }
        activate(&world, config)?;
        save(root, config)?;
        fs::remove_file(home.join("install-journal.json"))?;
        Ok(())
    })();
    if let Err(error) = operation {
        *config = previous;
        recover_one(root, config.id).context(
            "Could not restore the previous pack installation. Restart to retry recovery.",
        )?;
        return Err(error);
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct InstallJournal {
    previous: ServerConfig,
    activation_files: Vec<(String, Option<Vec<u8>>)>,
    created: Vec<(String, Uuid)>,
}

fn recover_one(root: &Path, id: Uuid) -> Result<()> {
    let home = home(root, id);
    let journal_path = home.join("install-journal.json");
    if !journal_path.exists() {
        return Ok(());
    }
    let journal: InstallJournal = serde_json::from_slice(&fs::read(&journal_path)?)?;
    ensure!(
        journal.previous.id == id,
        "Recovery journal belongs to a different server"
    );
    let world = home.join("worlds/world");
    private_dir(&world)?;
    for (name, bytes) in journal.activation_files {
        ensure!(
            ["world_behavior_packs.json", "world_resource_packs.json"].contains(&name.as_str()),
            "Invalid recovery activation filename"
        );
        let path = world.join(name);
        if let Some(bytes) = bytes {
            let mut temp = tempfile::NamedTempFile::new_in(&world)?;
            temp.write_all(&bytes)?;
            temp.as_file().sync_all()?;
            temp.persist(path).map_err(|e| e.error)?;
        } else if path.exists() {
            fs::remove_file(path)?;
        }
    }
    save(root, &journal.previous)?;
    for (kind, id) in journal.created {
        ensure!(
            ["behavior_packs", "resource_packs"].contains(&kind.as_str()),
            "Invalid recovery pack kind"
        );
        let path = home.join(kind).join(id.to_string());
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
    }
    fs::remove_file(journal_path)?;
    Ok(())
}

/// Recover only pending installations while the caller holds the application lease.
pub fn recover_installations(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root.join("servers"))? {
        let entry = entry?;
        if let Ok(id) = entry.file_name().to_string_lossy().parse() {
            recover_one(root, id)?;
        }
    }
    Ok(())
}

fn activate(world: &Path, config: &ServerConfig) -> Result<()> {
    for (kind, name) in [
        ("behavior_packs", "behavior"),
        ("resource_packs", "resource"),
    ] {
        let path = world.join(format!("world_{name}_packs.json"));
        let mut entries: Vec<serde_json::Value> = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)
                .context("Existing pack activation is not a JSON list")?
        } else {
            Vec::new()
        };
        for pack in config.addons.iter().flat_map(|a| &a.packs) {
            if pack.kind()? == kind {
                let id = pack.header.uuid.to_string();
                if let Some(existing) = entries
                    .iter()
                    .find(|p| p["pack_id"].as_str() == Some(id.as_str()))
                {
                    ensure!(
                        existing["version"] == serde_json::json!(pack.header.version),
                        "An active pack has a conflicting version"
                    );
                } else {
                    entries.push(serde_json::json!({"pack_id":pack.header.uuid,"version":pack.header.version}));
                }
            }
        }
        atomic_json(&path, &entries)?;
    }
    Ok(())
}

pub struct Managed {
    child: Child,
    input: ChildStdin,
    pub lines: Arc<Mutex<VecDeque<String>>>,
}
impl Managed {
    pub fn command(&mut self, command: &str) -> Result<()> {
        ensure!(
            !command.is_empty() && !command.contains(['\r', '\n', '\0']),
            "Enter one console command at a time"
        );
        writeln!(self.input, "{command}")?;
        self.input.flush()?;
        Ok(())
    }
    pub fn running(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }
    pub fn stop(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_some() {
            return Ok(());
        }
        let _ = self.command("stop");
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        self.child.kill()?;
        self.child.wait()?;
        Ok(())
    }
}
impl Drop for Managed {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
#[derive(Default, Clone)]
pub struct Processes {
    entries: Arc<Mutex<HashMap<Uuid, Managed>>>,
}
impl Processes {
    pub fn running(&self, id: Uuid) -> bool {
        self.entries
            .lock()
            .expect("process registry lock")
            .get_mut(&id)
            .is_some_and(Managed::running)
    }
    pub fn require_stopped(&self, id: Uuid) -> Result<()> {
        ensure!(
            !self.running(id),
            "Stop this server before changing files or creating a backup"
        );
        Ok(())
    }
    pub fn start(&self, root: &Path, config: &ServerConfig) -> Result<()> {
        ensure!(!self.running(config.id), "This server is already running");
        let home = home(root, config.id);
        ensure!(
            config.version.is_some() && home.join(BINARY).is_file(),
            "Install BDS before starting this server"
        );
        save(root, config)?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.join("console.log"))?;
        let mut command = Command::new(home.join(BINARY));
        command
            .current_dir(&home)
            .env("LD_LIBRARY_PATH", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .context("BDS could not start. Check that its runtime libraries are installed.")?;
        let input = child.stdin.take().context("Cannot open the BDS console")?;
        let lines = Arc::new(Mutex::new(VecDeque::new()));
        let file = Arc::new(Mutex::new(log));
        if let Some(out) = child.stdout.take() {
            capture(out, lines.clone(), file.clone());
        }
        if let Some(out) = child.stderr.take() {
            capture(out, lines.clone(), file);
        }
        self.entries.lock().expect("process registry lock").insert(
            config.id,
            Managed {
                child,
                input,
                lines,
            },
        );
        Ok(())
    }
    pub fn stop(&self, id: Uuid) -> Result<()> {
        self.entries
            .lock()
            .expect("process registry lock")
            .get_mut(&id)
            .context("This server is not running")?
            .stop()
    }
    pub fn stop_all(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            for process in entries.values_mut() {
                let _ = process.stop();
            }
        }
    }
    pub fn command(&self, id: Uuid, text: &str) -> Result<()> {
        ensure!(
            self.running(id),
            "Start the server before sending a command"
        );
        self.entries
            .lock()
            .expect("process registry lock")
            .get_mut(&id)
            .context("Server process disappeared")?
            .command(text)
    }
    pub fn logs(&self, id: Uuid) -> Vec<String> {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(&id).map(|e| e.lines.clone()))
            .and_then(|lines| lines.lock().ok().map(|l| l.iter().cloned().collect()))
            .unwrap_or_default()
    }
}

fn capture(
    reader: impl Read + Send + 'static,
    lines: Arc<Mutex<VecDeque<String>>>,
    file: Arc<Mutex<File>>,
) {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        loop {
            line.clear();
            match (&mut reader).take(64 * 1024).read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&line).trim_end().to_string();
            if let Ok(mut file) = file.lock() {
                let _ = writeln!(file, "{text}");
            }
            if let Ok(mut lines) = lines.lock() {
                lines.push_back(text);
                while lines.len() > 1200 {
                    lines.pop_front();
                }
            }
        }
    });
}

pub fn backup(root: &Path, config: &ServerConfig) -> Result<PathBuf> {
    let home = home(root, config.id);
    let destination = root.join("backups");
    private_dir(&destination)?;
    let mut temp = tempfile::NamedTempFile::new_in(&destination)?;
    {
        let mut zip = ZipWriter::new(temp.as_file_mut());
        fn append(zip: &mut ZipWriter<&mut File>, base: &Path, path: &Path) -> Result<()> {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let metadata = fs::symlink_metadata(entry.path())?;
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "Backup refuses symbolic links"
                );
                if metadata.is_dir() {
                    append(zip, base, &entry.path())?;
                } else if metadata.is_file() {
                    let name = entry
                        .path()
                        .strip_prefix(base)?
                        .to_string_lossy()
                        .replace('\\', "/");
                    if name == BINARY || name == "console.log" || name.starts_with("profiles/") {
                        continue;
                    }
                    zip.start_file(
                        name,
                        SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated),
                    )?;
                    std::io::copy(&mut File::open(entry.path())?, zip)?;
                }
            }
            Ok(())
        }
        append(&mut zip, &home, &home)?;
        zip.finish()?;
    }
    temp.as_file().sync_all()?;
    let path = destination.join(format!("{}-{}.zip", config.id, Uuid::new_v4()));
    temp.persist(&path).map_err(|e| e.error)?;
    Ok(path)
}
