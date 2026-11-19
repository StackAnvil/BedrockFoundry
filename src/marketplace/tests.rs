use super::*;
use crate::storage::Store;
use purchase::{PurchaseAttempt, TransactionFailure, parse_balance, transaction};
use reqwest::header::HeaderValue;
use std::io::{Read, Write};

const OFFER: &str = "11111111-1111-4111-8111-111111111111";
const PACK: &str = "22222222-2222-4222-8222-222222222222";
const CURRENCY: &str = "ecd19d3c-7635-402c-a185-eb11cb6c6946";

fn listing() -> Value {
    let price = json!({"Prices":[{"Amounts":[{"CurrencyId":CURRENCY,"Amount":490}]}]});
    json!({
        "Id":OFFER,"Title":{"NEUTRAL":"Fixture Add-On"},"Description":{"en-US":"A synthetic offer."},
        "ContentType":"MarketplaceDurableCatalog_V1.2","Tags":["addon"],
        "DisplayProperties":{"creatorName":"Fixture creator","price":490,"purchasable":true,
            "packIdentity":[{"uuid":PACK,"version":"1.0.0","type":"behaviorpack"}]},
        "Price":price,"PriceOptions":price
    })
}
fn item() -> CatalogItem {
    CatalogItem::parse(&listing(), &Ownership::default()).unwrap()
}
fn quote() -> PurchaseQuote {
    PurchaseQuote::new(item(), 1000, "a".repeat(64), 100).unwrap()
}

#[test]
fn receipt_ownership_accepts_null_offer_lists_and_preserves_active_offer_ids() {
    let receipt = json!({"EntityId":"e".repeat(32),"ReceiptData":{"DeviceId":"d".repeat(32)},"EntitlementReceipts":[
        {"PackId":PACK,"OfferIds":null,"Amount":1},
        {"PackId":PACK,"OfferIds":[OFFER],"Amount":1},
        {"PackId":"33333333-3333-4333-8333-333333333333","OfferIds":["44444444-4444-4444-8444-444444444444"],"Amount":1,"ExpirationDate":"2020-01-01T00:00:00Z"},
        {"PackId":"55555555-5555-4555-8555-555555555555","OfferIds":["66666666-6666-4666-8666-666666666666"],"Amount":0}
    ]});
    let encoded = STANDARD.encode(serde_json::to_vec(&receipt).unwrap());
    let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
    let owned = receipt_ownership(&encoded, now).unwrap();
    assert_eq!(owned.offers, HashSet::from([OFFER.parse().unwrap()]));
    assert_eq!(owned.packs, HashSet::from([PACK.parse().unwrap()]));
    assert!(receipt_keys(&encoded, now).unwrap().is_empty());
}

#[test]
fn catalog_keeps_worlds_viewable_without_classifying_their_behavior_packs_as_addons() {
    let mut world = listing();
    world["Tags"] = json!(["worldtemplate"]);
    let parsed = CatalogItem::parse(&world, &Ownership::default()).unwrap();
    assert_eq!(parsed.kind, "World");
    assert!(parsed.addon.is_none());
    assert!(!parsed.owned);
    let pack_owner = Ownership {
        packs: HashSet::from([PACK.parse().unwrap()]),
        ..Default::default()
    };
    assert!(CatalogItem::parse(&world, &pack_owner).unwrap().owned);
    PurchaseQuote::new(parsed, 1000, "a".repeat(64), 100)
        .expect_err("World listing allowed an add-on purchase quote");
    let mut persona = listing();
    persona["Tags"] = json!([]);
    persona["DisplayProperties"]["packIdentity"] = json!([]);
    assert!(
        !CatalogItem::parse(&persona, &Ownership::default())
            .unwrap()
            .owned
    );
    let owned = Ownership {
        offers: HashSet::from([OFFER.parse().unwrap()]),
        ..Default::default()
    };
    assert!(CatalogItem::parse(&persona, &owned).unwrap().owned);
}

#[test]
fn catalog_pages_use_service_offsets_and_keep_search_text_out_of_filters() {
    let query = CatalogQuery {
        search: "dragons' or true".into(),
        addons_only: true,
        skip: 24,
    };
    let body = query.body().unwrap();
    assert_eq!(body["Skip"], 24);
    assert_eq!(body["Top"], PAGE_SIZE);
    assert_eq!(body["OrderBy"], "creationDate desc, id asc");
    assert_eq!(body["Search"], "dragons' or true");
    assert!(!body["Filter"].as_str().unwrap().contains("dragons"));
    let page = CatalogPage::parse(
        &json!({"data":{"Count":50,"Items":[listing(),listing()]}}),
        query,
        &Ownership::default(),
    )
    .unwrap();
    assert_eq!(page.next_skip, Some(26));
    assert_eq!(page.items[0].title, "Fixture Add-On");
    let end = CatalogPage::parse(
        &json!({"data":{"Count":50,"Items":[]}}),
        CatalogQuery {
            skip: 50,
            ..Default::default()
        },
        &Ownership::default(),
    )
    .unwrap();
    assert!(end.next_skip.is_none());
    CatalogPage::parse(
        &json!({"data":{"Items":[]}}),
        CatalogQuery::default(),
        &Ownership::default(),
    )
    .expect_err("Catalog accepted a page without a total count");
}

#[test]
fn purchase_quotes_reject_changed_price_packs_account_expiry_and_insufficient_funds() {
    let q = quote();
    let current = item();
    q.validate(&current, 490, &"a".repeat(64), 101).unwrap();
    for changed in [
        {
            let mut i = current.clone();
            i.purchase_price = Some(310);
            i
        },
        {
            let mut i = current.clone();
            i.price = Some(660);
            i
        },
        {
            let mut i = current.clone();
            i.owned = true;
            i
        },
        {
            let mut i = current.clone();
            i.purchasable = false;
            i
        },
        {
            let mut i = current.clone();
            i.addon.as_mut().unwrap().packs[0].version = [2, 0, 0];
            i
        },
        {
            let mut i = current.clone();
            i.addon = None;
            i
        },
    ] {
        assert!(q.validate(&changed, 1000, &"a".repeat(64), 101).is_err());
    }
    assert!(q.validate(&current, 1000, &"b".repeat(64), 101).is_err());
    assert!(q.validate(&current, 1000, &"a".repeat(64), 400).is_err());
    assert!(
        q.validate(&current, 489, &"a".repeat(64), 101)
            .unwrap_err()
            .to_string()
            .starts_with("Not enough Minecoins")
    );
}

#[test]
fn minecoin_prices_and_balances_reject_other_currencies_and_ambiguous_amounts() {
    assert_eq!(item().purchase_price, Some(490));
    let mut wrong = listing();
    wrong["PriceOptions"]["Prices"][0]["Amounts"][0]["CurrencyId"] = json!("other-currency");
    assert!(
        CatalogItem::parse(&wrong, &Ownership::default())
            .unwrap()
            .purchase_price
            .is_none()
    );
    let mut sale = listing();
    sale["PriceOptions"]["Prices"][0]["Amounts"][0]["Amount"] = json!(310);
    assert!(
        CatalogItem::parse(&sale, &Ownership::default())
            .unwrap()
            .purchase_price
            .is_none()
    );
    for value in [json!(-1), json!(1.5), json!("490"), Value::Null] {
        parse_balance(
            &json!({"result":{"virtualCurrencyBalances":[{"type":"Minecoin","amount":value}]}}),
        )
        .expect_err("Balance accepted an invalid Minecoin amount");
    }
    assert_eq!(parse_balance(&json!({"result":{"virtualCurrencyBalances":[{"type":"Minecoin","amount":0},{"type":"Other","amount":10}]}})).unwrap(),0);
    parse_balance(&json!({"result":{"virtualCurrencyBalances":[{"type":"Minecoin","amount":1},{"type":"Minecoin","amount":2}]}})).expect_err("Balance accepted duplicate Minecoin entries");
}

#[test]
fn an_unconfirmed_purchase_blocks_the_same_account_and_offer_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let q = quote();
    let store = Store::open(Some(temp.path().into())).unwrap();
    let first = PurchaseAttempt::reserve(&store, &q).unwrap();
    drop(store);
    let reopened = Store::open(Some(temp.path().into())).unwrap();
    assert_eq!(
        PurchaseAttempt::read(&reopened, &q)
            .unwrap()
            .unwrap()
            .correlation,
        first.correlation
    );
    PurchaseAttempt::reserve(&reopened, &q)
        .expect_err("Duplicate attempt bypassed the persisted reservation");
    let other = PurchaseQuote::new(item(), 1000, "b".repeat(64), 100).unwrap();
    PurchaseAttempt::reserve(&reopened, &other).unwrap();
    PurchaseAttempt::clear(&reopened, &q.account, q.item.id).unwrap();
    assert!(PurchaseAttempt::read(&reopened, &q).unwrap().is_none());
    assert!(PurchaseAttempt::read(&reopened, &other).unwrap().is_some());
}

fn service(status: u16, body: Value) -> (String, std::thread::JoinHandle<Value>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let header_end = loop {
            let mut chunk = [0; 1024];
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
        };
        let header = String::from_utf8_lossy(&bytes[..header_end]).to_ascii_lowercase();
        let length: usize = header
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        while bytes.len() < header_end + length {
            let mut chunk = [0; 1024];
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
        }
        let request = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
        let encoded = serde_json::to_vec(&body).unwrap();
        write!(
            stream,
            "HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            encoded.len()
        )
        .unwrap();
        stream.write_all(&encoded).unwrap();
        request
    });
    (format!("http://{address}/purchase"), thread)
}

#[tokio::test]
async fn transaction_sends_the_confirmed_amount_and_does_not_call_a_mismatched_success_accepted() {
    let q = quote();
    let correlation = Uuid::new_v4();
    let http = Client::builder()
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let (url, thread) = service(
        200,
        json!({"result":{"offerId":OFFER,"transactionIds":["fixture-transaction"]}}),
    );
    transaction(
        &http,
        &url,
        HeaderValue::from_static("fixture-authorization"),
        &q,
        correlation,
    )
    .await
    .unwrap();
    let body = thread.join().unwrap();
    assert_eq!(body["OfferId"], OFFER);
    assert_eq!(
        body["VirtualCurrency"],
        json!({"Amount":"490","Type":"Minecoin"})
    );
    assert_eq!(body["StoreId"], "");
    assert_eq!(body["CustomTags"]["CorrelationId"], correlation.to_string());
    for (status, response, expected) in [
        (
            200,
            json!({"result":{"offerId":"different-offer","transactionIds":["fixture"]}}),
            TransactionFailure::Unknown,
        ),
        (
            500,
            json!({"message":"private-service-payload"}),
            TransactionFailure::Unknown,
        ),
        (
            400,
            json!({"message":"private-service-payload"}),
            TransactionFailure::Rejected(400),
        ),
    ] {
        let (url, thread) = service(status, response);
        let error = transaction(
            &http,
            &url,
            HeaderValue::from_static("fixture"),
            &q,
            correlation,
        )
        .await
        .unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("private-service-payload"));
        thread.join().unwrap();
    }
}
