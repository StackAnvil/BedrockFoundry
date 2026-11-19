//! Feeds arbitrary bytes to ZIP admission. Admission may reject any input, but an
//! accepted archive must only contain safe, unaliased member names within the
//! entry and expansion limits.
#![no_main]

use bedrock_foundry::{archive::read_zip, storage::safe_relative};
use bedrock_foundry_fuzz::{MAX_EXPANDED, assert_relative};
use libfuzzer_sys::fuzz_target;
use std::collections::HashSet;

fuzz_target!(|data: &[u8]| {
    let Ok(files) = read_zip(data) else {
        return;
    };
    assert!(files.len() <= 30_000, "accepted {} members", files.len());
    let mut folded = HashSet::new();
    let mut expanded = 0u64;
    for (name, content) in &files {
        // write_pack and import_distribution join the untrimmed member name.
        safe_relative(name).expect("read_zip accepted a name that extraction rejects");
        assert_relative(name);
        assert!(
            folded.insert(name.to_ascii_lowercase()),
            "accepted members that differ only by ASCII case: {name:?}"
        );
        expanded += content.len() as u64;
    }
    assert!(
        expanded <= MAX_EXPANDED,
        "accepted {expanded} expanded bytes"
    );
});
