//! Parses arbitrary download metadata: the BDS download link list and the
//! Marketplace catalog records that select an add-on download. Accepted URLs
//! must be accepted again after serialization.
#![no_main]

use bedrock_foundry::{
    marketplace::{
        CatalogItem, CatalogPage, CatalogQuery, Ownership, PAGE_SIZE, download_url, parse_addon,
        validate_asset_url,
    },
    server::distribution_link,
};
use libfuzzer_sys::fuzz_target;
use serde_json::{Value, json};
use std::collections::HashSet;

fuzz_target!(|data: &[u8]| {
    if let Ok((url, version)) = distribution_link(data) {
        let platform = if cfg!(windows) {
            "serverBedrockWindows"
        } else {
            "serverBedrockLinux"
        };
        let again = serde_json::to_vec(&json!({"result": {"links": [
            {"downloadType": platform, "downloadUrl": url.as_str()}
        ]}}))
        .expect("serialize link metadata");
        let reparsed = distribution_link(&again).expect("serialized BDS link was rejected");
        assert_eq!(
            reparsed,
            (url, version),
            "BDS link changed after serialization"
        );
    }

    let Ok(value) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    if let Ok(url) = download_url(&value) {
        let reparsed = validate_asset_url(url.as_str()).expect("serialized asset URL was rejected");
        assert_eq!(reparsed, url, "asset URL changed after serialization");
    }
    if let Ok(Some(addon)) = parse_addon(&value, &HashSet::new()) {
        assert!(!addon.packs.is_empty(), "add-on without packs");
    }
    let owned = Ownership::default();
    drop(CatalogItem::parse(&value, &owned));
    if let Ok(page) = CatalogPage::parse(&value, CatalogQuery::default(), &owned) {
        assert!(
            page.items.len() as u64 <= PAGE_SIZE,
            "page exceeds its size"
        );
        assert!(
            page.next_skip.is_none_or(|next| next < page.total),
            "next page past the total"
        );
    }
});
