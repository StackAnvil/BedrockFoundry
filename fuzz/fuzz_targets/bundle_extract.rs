//! Decodes an arbitrary Marketplace bundle with fixed entitlement keys, then
//! writes every decoded pack to a sandbox. No pack file may be written outside
//! its destination directory, and every written file must match the decoded pack.
#![no_main]

use bedrock_foundry::archive::{decode_bundle, write_pack};
use bedrock_foundry_fuzz::{Sandbox, assert_relative};
use libfuzzer_sys::fuzz_target;
use std::{collections::HashMap, fs};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Pack identities used by the seed corpus.
const PACK_A: Uuid = Uuid::from_u128(0x0f6e_3c1a_8f6b_4d2e_9a51_5b7c_1d2e_3f40);
const PACK_B: Uuid = Uuid::from_u128(0x7a2b_9c4d_1e3f_4a5b_8c6d_7e8f_9a0b_1c2d);

fuzz_target!(|data: &[u8]| {
    // The first byte selects which packs the catalog expects and which packs
    // the account owns. The rest is the downloaded bundle.
    let Some((&flags, bundle)) = data.split_first() else {
        return;
    };
    let mut expected = HashMap::from([(PACK_A, [1, 0, 0])]);
    if flags & 1 != 0 {
        expected.insert(PACK_B, [1, 0, 0]);
    }
    let mut keys = HashMap::from([(PACK_A, Zeroizing::new(vec![0x51; 32]))]);
    if flags & 2 == 0 {
        keys.insert(PACK_B, Zeroizing::new(vec![0x52; 32]));
    }
    let Ok(packs) = decode_bundle(bundle, &keys, &expected) else {
        return;
    };
    assert_eq!(
        packs.len(),
        expected.len(),
        "decoded a different pack count"
    );

    let sandbox = Sandbox::new();
    let mut destinations = Vec::new();
    for pack in &packs {
        let destination = sandbox.dest.join(pack.manifest.header.uuid.to_string());
        destinations.push(destination.clone());
        for name in pack.files.keys() {
            assert_relative(name);
        }
        let written = write_pack(pack, &destination);
        sandbox.assert_contained(&destinations);
        // A file and a directory with the same path cannot both be written.
        if written.is_ok() {
            for (name, content) in &pack.files {
                let on_disk = fs::read(destination.join(name)).expect("read extracted file");
                assert_eq!(
                    &on_disk, content,
                    "extracted {name:?} differs from the pack"
                );
            }
        }
    }
});
