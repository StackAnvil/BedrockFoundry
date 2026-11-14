use aes::cipher::{AsyncStreamCipher, KeyIvInit};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bedrock_foundry::{
    archive::{self, DecodedPack, Manifest},
    marketplace::{self, Addon, PackIdentity},
    server::{self, ServerConfig},
    storage::{Store, safe_relative},
};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap},
    io::{Cursor, Read, Write},
};
use time::OffsetDateTime;
use uuid::Uuid;
use zeroize::Zeroizing;
use zip::write::SimpleFileOptions;

fn archive(entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn manifest(id: Uuid, kind: &str) -> Manifest {
    serde_json::from_value(json!({"header":{"uuid":id,"version":[1,0,0],"name":"Fixture","min_engine_version":[1,21,60]},"modules":[{"type":kind}],"dependencies":[]})).unwrap()
}
fn encrypted_pack(id: Uuid, key: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let content_key = b"0123456789abcdefghijklmnopqrstuv";
    let plaintext = br#"{"minecraft:block":{"description":{"identifier":"test:custom"}}}"#.to_vec();
    let mut encrypted = plaintext.clone();
    cfb8::Encryptor::<aes::Aes256>::new_from_slices(content_key, &content_key[..16])
        .unwrap()
        .encrypt(&mut encrypted);
    let mut index=serde_json::to_vec(&json!({"content":[{"path":"blocks/custom.json","key":std::str::from_utf8(content_key).unwrap()}]})).unwrap();
    cfb8::Encryptor::<aes::Aes256>::new_from_slices(key, &key[..16])
        .unwrap()
        .encrypt(&mut index);
    let mut header = vec![0u8; 256];
    header[4..8].copy_from_slice(&[0xfc, 0xb9, 0xcf, 0x9b]);
    header[16] = 36;
    header[17..53].copy_from_slice(id.to_string().as_bytes());
    header.extend(index);
    let m=serde_json::to_vec(&json!({"format_version":2,"header":{"uuid":id,"version":[1,0,0],"name":"Fixture","min_engine_version":[1,21,60]},"modules":[{"type":"data"}]})).unwrap();
    (
        archive(vec![
            ("manifest.json", m),
            ("contents.json", header),
            ("blocks/custom.json", encrypted),
        ]),
        plaintext,
    )
}
#[test]
fn decrypts_an_owned_nested_pack_and_preserves_its_payload() {
    let id = Uuid::new_v4();
    let key = [0x51; 32];
    let (pack, plaintext) = encrypted_pack(id, &key);
    let bundle = archive(vec![("ppack0.zip", pack)]);
    let decoded = archive::decode_bundle(
        &bundle,
        &HashMap::from([(id, Zeroizing::new(key.to_vec()))]),
        &HashMap::from([(id, [1, 0, 0])]),
    )
    .unwrap();
    assert_eq!(decoded[0].files["blocks/custom.json"], plaintext);
    assert!(!decoded[0].files.contains_key("contents.json"));
}
#[test]
fn rejects_a_pack_without_its_entitlement_or_with_a_different_catalog_version() {
    let id = Uuid::new_v4();
    let (pack, _) = encrypted_pack(id, &[0x51; 32]);
    assert!(
        archive::decode_bundle(&pack, &HashMap::new(), &HashMap::from([(id, [1, 0, 0])])).is_err()
    );
    assert!(
        archive::decode_bundle(
            &pack,
            &HashMap::from([(id, Zeroizing::new(vec![0x51; 32]))]),
            &HashMap::from([(id, [2, 0, 0])])
        )
        .is_err()
    );
}
#[test]
fn rejects_archive_paths_that_escape_or_alias_a_destination() {
    for path in [
        "../secret",
        "/absolute",
        "C:/Windows/file",
        "folder\\file",
        "a/../file",
        "a/./file",
        "a//file",
        ".",
        "CON",
        "aux.txt",
        "folder/COM1",
        "file.",
        "file ",
    ] {
        assert!(safe_relative(path).is_err(), "accepted {path}");
    }
    assert!(archive::read_zip(&archive(vec![("../secret", vec![1])])).is_err());
}
#[test]
fn rejects_duplicate_archive_payloads() {
    // ZIP writers refuse duplicate names. Model an external archive with two
    // equal-length member names patched in both local and central headers.
    let mut bytes = archive(vec![("same", vec![1]), ("diff", vec![2])]);
    let offsets: Vec<_> = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(i, name)| (name == b"diff").then_some(i))
        .collect();
    for offset in offsets {
        bytes[offset..offset + 4].copy_from_slice(b"same");
    }
    assert!(archive::read_zip(&bytes).is_err());
}

#[test]
fn ignores_expired_and_future_entitlements() {
    let id = Uuid::new_v4();
    let future = Uuid::new_v4();
    let valid = Uuid::new_v4();
    let entity = "e".repeat(32);
    let device = "d".repeat(32);
    let key = vec![0x6b; 32];
    let mut wrapped = vec![0; 64];
    for i in 0..32 {
        wrapped[2 * i] = key[i] ^ b'e' ^ b'd';
    }
    let content = STANDARD.encode(wrapped);
    let receipt = json!({"EntityId":entity,"ReceiptData":{"DeviceId":device},"EntitlementReceipts":[
        {"PackId":id,"Amount":1,"ContentKey":content,"ExpirationDate":"2020-01-01T00:00:00Z"},
        {"PackId":future,"Amount":1,"ContentKey":content,"StartDate":"2099-01-01T00:00:00Z"},
        {"PackId":valid,"Amount":1,"ContentKey":content,"StartDate":"0001-01-01T00:00:00"}]});
    let keys = marketplace::receipt_keys(
        &STANDARD.encode(serde_json::to_vec(&receipt).unwrap()),
        OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap(),
    )
    .unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[&valid].as_slice(), key);
}
#[test]
fn refuses_credentials_and_arbitrary_hosts_in_catalog_downloads() {
    for url in [
        "http://xforgeassets001.xboxlive.com/a",
        "https://xforgeassets001.xboxlive.com.attacker.test/a",
        "https://user:password@xforgeassets001.xboxlive.com/a",
        "https://127.0.0.1/a",
        "https://xforgeassets001.xboxlive.com:444/a",
        "https://xforgeassets001.xboxlive.com/a#fragment",
    ] {
        assert!(
            marketplace::validate_asset_url(url).is_err(),
            "accepted {url}"
        );
    }
    assert!(
        marketplace::validate_asset_url("https://xforgeassets001.xboxlive.com/asset.zip").is_ok()
    );
}
#[test]
fn preserves_custom_server_properties_and_prevents_line_injection() {
    let mut config = ServerConfig::new("Friends".into(), 19132).unwrap();
    let properties = config
        .properties("# user comment\nlevel-seed=12345\nserver-port=9999\nserver-port=10000\n")
        .unwrap();
    assert!(properties.contains("level-seed=12345\n"));
    assert_eq!(properties.matches("server-port=").count(), 1);
    assert!(properties.contains("online-mode=true\n"));
    assert!(properties.contains("transport=nethernet\n"));
    config.name = "Injected\nonline-mode=false".into();
    assert!(config.properties("").is_err());
}
#[test]
fn installs_both_pack_types_with_matching_world_activation_and_keeps_world_data() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(Some(temp.path().to_path_buf())).unwrap();
    let mut config = ServerConfig::new("Friends".into(), 19132).unwrap();
    config.version = Some("1.26.60.6".into());
    server::save(&store.root, &config).unwrap();
    let bp = Uuid::new_v4();
    let rp = Uuid::new_v4();
    let mut behavior = manifest(bp, "data");
    behavior
        .dependencies
        .push(serde_json::from_value(json!({"uuid":rp,"version":[1,0,0]})).unwrap());
    let packs = vec![
        DecodedPack {
            manifest: behavior,
            files: BTreeMap::from([("manifest.json".into(), b"original manifest bytes".to_vec())]),
        },
        DecodedPack {
            manifest: manifest(rp, "resources"),
            files: BTreeMap::from([("manifest.json".into(), b"resource bytes".to_vec())]),
        },
    ];
    let addon = Addon {
        id: Uuid::new_v4(),
        name: "Fixture".into(),
        creator: "Fixture creator".into(),
        owned: true,
        price: Some(0),
        packs: vec![
            PackIdentity {
                uuid: bp,
                version: [1, 0, 0],
                kind: "behaviorpack".into(),
            },
            PackIdentity {
                uuid: rp,
                version: [1, 0, 0],
                kind: "resourcepack".into(),
            },
        ],
    };
    let world = server::home(&store.root, config.id).join("worlds/world");
    std::fs::create_dir_all(&world).unwrap();
    std::fs::write(world.join("level.dat"), b"existing-world").unwrap();
    server::install_addon(&store.root, &mut config, &addon, &packs).unwrap();
    let active: serde_json::Value =
        serde_json::from_slice(&std::fs::read(world.join("world_behavior_packs.json")).unwrap())
            .unwrap();
    assert_eq!(active, json!([{"pack_id":bp,"version":[1,0,0]}]));
    let active: serde_json::Value =
        serde_json::from_slice(&std::fs::read(world.join("world_resource_packs.json")).unwrap())
            .unwrap();
    assert_eq!(active, json!([{"pack_id":rp,"version":[1,0,0]}]));
    assert_eq!(
        std::fs::read(world.join("level.dat")).unwrap(),
        b"existing-world"
    );
    assert_eq!(
        std::fs::read(
            server::home(&store.root, config.id)
                .join("behavior_packs")
                .join(bp.to_string())
                .join("manifest.json")
        )
        .unwrap(),
        b"original manifest bytes"
    );
    assert!(server::install_addon(&store.root, &mut config, &addon, &packs).is_err());
}
#[test]
fn missing_pack_dependency_leaves_server_configuration_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(Some(temp.path().to_path_buf())).unwrap();
    let mut config = ServerConfig::new("Friends".into(), 19132).unwrap();
    config.version = Some("1.26.60.6".into());
    server::save(&store.root, &config).unwrap();
    let id = Uuid::new_v4();
    let mut m = manifest(id, "data");
    m.dependencies
        .push(serde_json::from_value(json!({"uuid":Uuid::new_v4(),"version":[1,0,0]})).unwrap());
    let addon = Addon {
        id: Uuid::new_v4(),
        name: "Fixture".into(),
        creator: "Fixture".into(),
        owned: true,
        price: Some(0),
        packs: vec![],
    };
    assert!(
        server::install_addon(
            &store.root,
            &mut config,
            &addon,
            &[DecodedPack {
                manifest: m,
                files: BTreeMap::new()
            }]
        )
        .is_err()
    );
    assert!(config.addons.is_empty());
    assert!(server::list(&store.root).unwrap()[0].addons.is_empty());
}
#[test]
fn backup_includes_world_settings_and_addon_files() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(Some(temp.path().to_path_buf())).unwrap();
    let config = ServerConfig::new("Friends".into(), 19132).unwrap();
    server::save(&store.root, &config).unwrap();
    let home = server::home(&store.root, config.id);
    std::fs::create_dir_all(home.join("worlds/world")).unwrap();
    std::fs::write(home.join("worlds/world/level.dat"), b"world-state").unwrap();
    let archive = server::backup(&store.root, &config).unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).unwrap()).unwrap();
    let mut bytes = Vec::new();
    zip.by_name("worlds/world/level.dat")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"world-state");
    assert!(zip.by_name("foundry.json").is_ok());
}
#[test]
fn another_process_cannot_open_the_same_application_store() {
    let temp = tempfile::tempdir().unwrap();
    let first = Store::open(Some(temp.path().to_path_buf())).unwrap();
    assert!(Store::open(Some(temp.path().to_path_buf())).is_err());
    drop(first);
    assert!(Store::open(Some(temp.path().to_path_buf())).is_ok());
}

#[test]
fn interrupted_pack_installation_restores_the_exact_previous_world_activation() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(Some(temp.path().to_path_buf())).unwrap();
    let config = ServerConfig::new("Friends".into(), 19132).unwrap();
    server::save(&store.root, &config).unwrap();
    let home = server::home(&store.root, config.id);
    let world = home.join("worlds/world");
    std::fs::create_dir_all(&world).unwrap();
    let previous = b"[ { \"pack_id\": \"external\", \"version\": [1,0,0] } ]\n";
    std::fs::write(world.join("world_behavior_packs.json"), b"[]").unwrap();
    let pack = Uuid::new_v4();
    std::fs::create_dir_all(home.join("behavior_packs").join(pack.to_string())).unwrap();
    std::fs::write(home.join("install-journal.json"),serde_json::to_vec(&json!({"previous":config,"activation_files":[["world_behavior_packs.json",previous.to_vec()],["world_resource_packs.json",null]],"created":[["behavior_packs",pack]]})).unwrap()).unwrap();
    server::recover_installations(&store.root).unwrap();
    assert_eq!(
        std::fs::read(world.join("world_behavior_packs.json")).unwrap(),
        previous
    );
    assert!(!home.join("behavior_packs").join(pack.to_string()).exists());
    assert!(!home.join("install-journal.json").exists());
}

#[test]
fn installing_an_addon_keeps_an_externally_managed_pack_active() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(Some(temp.path().to_path_buf())).unwrap();
    let mut config = ServerConfig::new("Friends".into(), 19132).unwrap();
    config.version = Some("1.26.60.6".into());
    server::save(&store.root, &config).unwrap();
    let world = server::home(&store.root, config.id).join("worlds/world");
    std::fs::create_dir_all(&world).unwrap();
    let external = Uuid::new_v4();
    let previous = json!([{"pack_id":external,"version":[2,3,4]}]);
    std::fs::write(
        world.join("world_behavior_packs.json"),
        serde_json::to_vec(&previous).unwrap(),
    )
    .unwrap();
    let pack = Uuid::new_v4();
    let m = manifest(pack, "data");
    let addon = Addon {
        id: Uuid::new_v4(),
        name: "Fixture".into(),
        creator: "Fixture".into(),
        owned: true,
        price: Some(0),
        packs: vec![],
    };
    server::install_addon(
        &store.root,
        &mut config,
        &addon,
        &[DecodedPack {
            manifest: m,
            files: BTreeMap::new(),
        }],
    )
    .unwrap();
    let entries: serde_json::Value =
        serde_json::from_slice(&std::fs::read(world.join("world_behavior_packs.json")).unwrap())
            .unwrap();
    assert_eq!(entries[0], previous[0]);
    assert_eq!(entries.as_array().unwrap().len(), 2);
}

#[test]
fn network_access_uses_the_native_bind_property_and_a_fixed_udp_port() {
    let mut config = ServerConfig::new("Friends".into(), 19132).unwrap();
    let private = config.properties("server-bind-address=0.0.0.0\n").unwrap();
    assert!(private.contains("server-ip=127.0.0.1\n"));
    assert!(private.contains("server-udp-ports=19133\n"));
    assert!(!private.contains("server-bind-address="));
    config.public = true;
    config.public_address = Some("203.0.113.10".parse().unwrap());
    let public = config.properties("").unwrap();
    assert!(public.contains("server-ip=0.0.0.0\n"));
    assert!(public.contains("server-udp-ports=203.0.113.10:19133:19133\n"));
}

#[cfg(unix)]
#[test]
fn stopping_a_managed_server_sends_stop_and_waits_for_its_exit() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(Some(temp.path().to_path_buf())).unwrap();
    let mut config = ServerConfig::new("Lifecycle fixture".into(), 19132).unwrap();
    config.version = Some("1.26.60.6".into());
    server::save(&store.root, &config).unwrap();
    let home = server::home(&store.root, config.id);
    std::fs::write(home.join("bedrock_server"),b"#!/bin/sh\nprintf 'Server started.\\n'\nwhile read -r command; do\n  if [ \"$command\" = stop ]; then printf 'graceful exit' > stopped; exit 0; fi\ndone\n").unwrap();
    std::fs::set_permissions(
        home.join("bedrock_server"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let processes = server::Processes::default();
    processes.start(&store.root, &config).unwrap();
    assert!(processes.require_stopped(config.id).is_err());
    processes.clone().stop_all();
    assert!(!processes.running(config.id));
    assert_eq!(
        std::fs::read(home.join("stopped")).unwrap(),
        b"graceful exit"
    );
}
