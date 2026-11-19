//! Parses arbitrary entitlement receipts. Key extraction and ownership use the
//! same receipt and activity rules, so every pack that yields a content key must
//! also count as owned, and a receipt that ownership rejects must not yield keys.
#![no_main]

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bedrock_foundry::marketplace::{receipt_keys, receipt_ownership};
use libfuzzer_sys::fuzz_target;
use time::OffsetDateTime;

fuzz_target!(|data: &[u8]| {
    // The service sends base64-encoded JSON. Mode 0 passes the input through as
    // the encoded receipt text. Other modes encode the input as the JSON body.
    let Some((&mode, body)) = data.split_first() else {
        return;
    };
    let encoded = if mode == 0 {
        let Ok(text) = std::str::from_utf8(body) else {
            return;
        };
        text.to_owned()
    } else {
        STANDARD.encode(body)
    };
    let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).expect("fixed time");
    let keys = receipt_keys(&encoded, now);
    let ownership = receipt_ownership(&encoded, now);
    match (keys, ownership) {
        (Ok(keys), Ok(ownership)) => {
            for (pack, key) in &keys {
                assert_eq!(key.len(), 32, "content key for {pack} has the wrong size");
                assert!(
                    ownership.packs.contains(pack),
                    "key for unowned pack {pack}"
                );
            }
        }
        (Ok(_), Err(error)) => panic!("keys extracted from a rejected receipt: {error:#}"),
        (Err(_), _) => {}
    }
});
