use crate::storage::{private_dir, safe_relative};
use aes::cipher::{AsyncStreamCipher, KeyIvInit};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io::{Cursor, Read},
    path::Path,
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub const MAX_ARCHIVE: usize = 256 * 1024 * 1024;
const MAX_EXPANDED: u64 = 512 * 1024 * 1024;
const MAX_FILES: usize = 30_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PackHeader {
    pub uuid: Uuid,
    pub version: [u32; 3],
    pub name: String,
    #[serde(default)]
    pub min_engine_version: Option<[u32; 3]>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Module {
    pub r#type: String,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Dependency {
    pub uuid: Option<Uuid>,
    pub version: serde_json::Value,
    pub module_name: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub header: PackHeader,
    pub modules: Vec<Module>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}
#[derive(Clone)]
pub struct DecodedPack {
    pub manifest: Manifest,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Manifest {
    pub fn kind(&self) -> Result<&'static str> {
        let behavior = self
            .modules
            .iter()
            .any(|m| matches!(m.r#type.as_str(), "data" | "script"));
        let resource = self.modules.iter().any(|m| m.r#type == "resources");
        match (behavior, resource) {
            (true, false) => Ok("behavior_packs"),
            (false, true) => Ok("resource_packs"),
            _ => bail!("Unsupported or mixed pack module types"),
        }
    }
}

pub fn read_zip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(
        bytes.len() <= MAX_ARCHIVE,
        "Archive exceeds the download size limit"
    );
    let mut budget = 0;
    read_zip_with_budget(bytes, &mut budget)
}
fn read_zip_with_budget(bytes: &[u8], budget: &mut u64) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(
        bytes.len() <= MAX_ARCHIVE,
        "Archive exceeds the download size limit"
    );
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    // zip's directory map retains only the last entry for duplicate names.
    // Inspect the accepted central directory before that loss of information.
    let mut position = usize::try_from(zip.central_directory_start())?;
    let mut names = std::collections::HashSet::new();
    let mut central_entries = 0usize;
    let mut legacy_indices = 0usize;
    while bytes.get(position..position + 4) == Some(&[0x50, 0x4b, 0x01, 0x02]) {
        let header = bytes
            .get(position..position + 46)
            .context("Truncated ZIP directory")?;
        let length = |offset| u16::from_le_bytes([header[offset], header[offset + 1]]) as usize;
        let name_len = length(28);
        let name = bytes
            .get(position + 46..position + 46 + name_len)
            .context("Truncated ZIP filename")?;
        central_entries += 1;
        ensure!(
            central_entries <= MAX_FILES,
            "Archive contains too many entries"
        );
        if !names.insert(name.iter().map(u8::to_ascii_lowercase).collect::<Vec<_>>()) {
            ensure!(name == b"contents.json", "Archive contains duplicate files");
            legacy_indices += 1;
        }
        position = position
            .checked_add(46 + name_len + length(30) + length(32))
            .context("ZIP directory overflow")?;
    }
    ensure!(
        central_entries - legacy_indices == zip.len(),
        "ZIP directory has ambiguous file names"
    );
    ensure!(zip.len() <= MAX_FILES, "Archive contains too many files");

    let mut files = BTreeMap::new();
    for n in 0..zip.len() {
        let mut file = zip.by_index(n)?;
        let name = file.name().to_owned();
        safe_relative(name.trim_end_matches('/'))?;
        ensure!(
            file.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
            "Archive contains a symbolic link"
        );
        if file.is_dir() {
            continue;
        }
        *budget = budget
            .checked_add(file.size())
            .context("Archive expansion overflow")?;
        ensure!(
            *budget <= MAX_EXPANDED,
            "Archive exceeds the extraction size limit"
        );
        let mut content = Vec::new();
        (&mut file)
            .take(MAX_EXPANDED + 1)
            .read_to_end(&mut content)?;
        ensure!(
            content.len() as u64 <= MAX_EXPANDED,
            "Archive member exceeds the extraction size limit"
        );
        if files.insert(name.clone(), content).is_some() && name != "contents.json" {
            bail!("Archive contains duplicate files");
        }
    }
    Ok(files)
}

pub fn decrypt(data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    ensure!(key.len() == 32, "Invalid Marketplace content key length");
    let mut result = data.to_vec();
    cfb8::Decryptor::<aes::Aes256>::new_from_slices(key, &key[..16])
        .map_err(|_| anyhow::anyhow!("Invalid Marketplace cipher parameters"))?
        .decrypt(&mut result);
    Ok(result)
}

pub fn decode_bundle(
    bytes: &[u8],
    keys: &HashMap<Uuid, Zeroizing<Vec<u8>>>,
    expected: &HashMap<Uuid, [u32; 3]>,
) -> Result<Vec<DecodedPack>> {
    let mut budget = 0;
    let outer = read_zip_with_budget(bytes, &mut budget)?;
    let archives = if outer.contains_key("manifest.json") {
        vec![outer]
    } else {
        ensure!(outer.len() <= 16, "Add-on contains too many nested packs");
        let mut packs = Vec::new();
        for (name, bytes) in outer {
            ensure!(
                name.ends_with(".zip") || name.ends_with(".mcpack"),
                "Unsupported add-on archive wrapper"
            );
            packs.push(read_zip_with_budget(&bytes, &mut budget)?);
        }
        packs
    };
    let mut packs = Vec::new();
    for files in archives {
        let manifest_bytes = files
            .get("manifest.json")
            .context("Pack has no manifest.json")?;
        let manifest: Manifest =
            serde_json::from_slice(manifest_bytes).context("Invalid pack manifest")?;
        manifest.kind()?;
        let identity = manifest.header.uuid;
        ensure!(
            expected.get(&identity) == Some(&manifest.header.version),
            "Downloaded pack identity or version does not match the catalog"
        );
        ensure!(
            !packs
                .iter()
                .any(|p: &DecodedPack| p.manifest.header.uuid == identity),
            "Duplicate pack identity in add-on"
        );
        let key = keys
            .get(&identity)
            .context("This account has no current entitlement key for this pack")?;
        let mut decoded = BTreeMap::new();
        if let Some(index) = files.get("contents.json") {
            let id = identity.to_string();
            ensure!(
                index.len() >= 256
                    && index[4..8] == [0xfc, 0xb9, 0xcf, 0x9b]
                    && index[16] == 36
                    && index[17..53] == *id.as_bytes(),
                "Invalid encrypted pack index"
            );
            #[derive(Deserialize)]
            struct Index {
                content: Vec<Content>,
            }
            #[derive(Deserialize)]
            struct Content {
                path: String,
                key: Option<String>,
            }
            let index: Index = serde_json::from_slice(&decrypt(&index[256..], key)?)
                .context("The account key cannot decrypt this pack index")?;
            for entry in index.content {
                safe_relative(entry.path.trim_end_matches('/'))?;
                if entry.path.ends_with('/')
                    || matches!(entry.path.as_str(), "contents.json" | "signatures.json")
                {
                    continue;
                }
                let bytes = files
                    .get(&entry.path)
                    .context("Pack index refers to a missing file")?;
                let bytes = if let Some(key) = entry.key.filter(|k| !k.is_empty()) {
                    let key: Option<Vec<u8>> =
                        key.chars().map(|c| u8::try_from(c as u32).ok()).collect();
                    decrypt(bytes, &key.context("Invalid per-file content key")?)?
                } else {
                    bytes.clone()
                };
                ensure!(
                    decoded.insert(entry.path, bytes).is_none(),
                    "Pack index contains duplicate paths"
                );
            }
        } else {
            decoded = files.clone();
        }
        decoded.insert("manifest.json".into(), manifest_bytes.clone());
        packs.push(DecodedPack {
            manifest,
            files: decoded,
        });
    }
    ensure!(
        packs.len() == expected.len(),
        "Download is missing one or more catalog packs"
    );
    Ok(packs)
}

pub fn write_pack(pack: &DecodedPack, destination: &Path) -> Result<()> {
    private_dir(destination)?;
    for (name, bytes) in &pack.files {
        let path = destination.join(safe_relative(name)?);
        private_dir(path.parent().context("Pack path has no parent")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(())
}
