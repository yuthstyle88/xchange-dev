//! Written before the implementation (#3473, up-front tests by claude/studio-claude-3 on the
//! xchange BE controller claude/a90ab16c's order). Do not edit, delete or weaken.
//!
//! `POST /api/kiosk` `get` tells the desk which quote the request holds (`quoteId`), so the desk
//! can complete the request through `POST /api/transactions` with that quote
//! (`docs/design/blueprint/quote-transaction.md` §4.6, `:383-386`). `quoteId` is the request's
//! `kiosk_requests.quote_id` as lower-case UUID text, or JSON `null` when the column is NULL.
//!
//! One test per row of the ticket's §9. The end-to-end row (`get` then `POST /api/transactions`
//! with that `quoteId`) is held by the first test here plus G7's own tests (`tx4_create_test.rs`,
//! `kiosk_save_step_test.rs`): the create side already accepts the request's own quote; what is
//! missing is only the read side tested here.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::{ACCEPT_FIRST, Counter, person};
use serde_json::{Value, json};

const FIRST: &str = "00000000-0000-4000-8000-0000000c3471";
const SECOND: &str = "00000000-0000-4000-8000-0000000c3472";

/// The fields `get` answers today, in today's order (`dto/kiosk.rs` `get_json`); `quoteId` is the
/// one key the ticket adds.
const TODAYS_KEYS: [&str; 10] = [
    "paymentMethod",
    "type",
    "currency",
    "rate",
    "foreign",
    "thb",
    "inputAmount",
    "branchName",
    "id",
    "queue",
];

/// A held request: quoted, confirmed into the queue and accepted by `who`.
async fn held(counter: &Counter, who: &str, id: &str) {
    counter.waiting_request(id).await;
    let accepted = counter.ask(Some(who), "accept", json!({ "id": id })).await;
    assert_eq!(accepted.status, 200, "accept: {}", accepted.text);
}

/// The request's stored quote, as the column holds it.
async fn stored_quote(counter: &Counter, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT quote_id::text FROM kiosk_requests WHERE id = $1")
        .bind(id)
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("read kiosk_requests.quote_id")
}

/// The answer's keys in the order they were written.
fn keys_in_order(text: &str) -> Vec<String> {
    let _: serde_json::Map<String, Value> =
        serde_json::from_str(text).unwrap_or_else(|e| panic!("a JSON object ({e}): {text}"));
    text.trim()
        .trim_matches(['{', '}'])
        .split(',')
        .map(|entry| {
            let (k, _) = entry
                .split_once(':')
                .unwrap_or_else(|| panic!("key:value in {text}"));
            serde_json::from_str::<String>(k.trim())
                .unwrap_or_else(|e| panic!("key {k} ({e}) in {text}"))
        })
        .collect()
}

async fn open() -> Counter {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    open_counter(&counter.desk.db.pool, &fr1.id).await;
    counter
}

/// `accept` starts the request's queue ticket, and the queue serves only a teller with an ACTIVE
/// counter: seeded as `kiosk_http_test.rs` `open_counters` seeds it.
async fn open_counter(pool: &sqlx::PgPool, teller: &str) {
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         SELECT $2::uuid, organization_id, 'Teller', 'ACTIVE', 'test' FROM branches WHERE id = $1 \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(rpc_common::MAIN)
    .bind(teller)
    .execute(pool)
    .await
    .expect("the teller's user row");
    sqlx::query(
        "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
         SELECT organization_id, id, $2::uuid, 'ACTIVE' FROM branches WHERE id = $1 \
         ON CONFLICT (branch_id, teller_id) DO NOTHING",
    )
    .bind(rpc_common::MAIN)
    .bind(teller)
    .execute(pool)
    .await
    .expect("open the teller's counter");
}

/// §9 row 1: a request quoted through `QuoteService` (TX-7) answers `quoteId` = its
/// `kiosk_requests.quote_id`.
#[actix_web::test]
async fn get_answers_the_requests_own_quote_id() {
    let counter = open().await;
    held(&counter, "FR1", FIRST).await;
    let quote_id = stored_quote(&counter, FIRST)
        .await
        .expect("setup: the kiosk's quote stored the request's quote_id (TX-7)");

    let got = counter
        .ask(Some("FR1"), "get", json!({ "id": FIRST }))
        .await;
    assert_eq!(got.status, 200, "{}", got.text);
    assert_eq!(
        got.json()["quoteId"],
        json!(quote_id.to_lowercase()),
        "`quoteId` is the request's stored quote: {}",
        got.text
    );
    counter.finish().await;
}

/// §9 row 2: a request whose `quote_id` is NULL (made before TX-7, or no quote was returned)
/// answers `quoteId: null`, present, not missing, with every other field unchanged.
#[actix_web::test]
async fn a_request_with_no_stored_quote_answers_quote_id_null() {
    let counter = open().await;
    held(&counter, "FR1", FIRST).await;
    let before = counter
        .ask(Some("FR1"), "get", json!({ "id": FIRST }))
        .await;
    sqlx::query("UPDATE kiosk_requests SET quote_id = NULL WHERE id = $1")
        .bind(FIRST)
        .execute(&counter.desk.db.pool)
        .await
        .expect("setup: a request with no quote");

    let got = counter
        .ask(Some("FR1"), "get", json!({ "id": FIRST }))
        .await;
    assert_eq!(got.status, 200, "{}", got.text);
    let answer = got.json();
    assert!(
        answer
            .as_object()
            .is_some_and(|o| o.contains_key("quoteId")),
        "`quoteId` is written even when there is none: {}",
        got.text
    );
    assert_eq!(answer["quoteId"], Value::Null, "{}", got.text);
    let mut rest = answer.clone();
    let mut was = before.json();
    for value in [&mut rest, &mut was] {
        value.as_object_mut().expect("an object").remove("quoteId");
    }
    assert_eq!(rest, was, "every other field is unchanged");
    counter.finish().await;
}

/// §9 row 3: another branch's request keeps `service.get`'s refusal, and no `quoteId` is read
/// from the other branch.
#[actix_web::test]
async fn another_branchs_request_is_refused_and_gives_no_quote_id() {
    let counter = open().await;
    held(&counter, "FR1", FIRST).await;
    assert!(
        stored_quote(&counter, FIRST).await.is_some(),
        "setup: quoted"
    );
    let pool = &counter.desk.db.pool;
    sqlx::query(
        "INSERT INTO branches (id, organization_id, name) \
         SELECT 'OTHER', organization_id, 'Other branch' FROM branches WHERE id = $1",
    )
    .bind(rpc_common::MAIN)
    .execute(pool)
    .await
    .expect("setup: a second branch");
    // The same request, quote included, copied onto the other branch under a new id: the queue
    // ticket of the original keeps it on MAIN.
    let mut tx = pool.begin().await.expect("begin");
    for sql in [
        "CREATE TEMP TABLE other_request ON COMMIT DROP AS SELECT * FROM kiosk_requests WHERE id = $1",
        "UPDATE other_request SET id = $2, branch_id = 'OTHER'",
        "INSERT INTO kiosk_requests SELECT * FROM other_request",
    ] {
        sqlx::query(sql)
            .bind(FIRST)
            .bind(SECOND)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("setup: the other branch's request ({sql}): {e}"));
    }
    tx.commit().await.expect("commit");
    assert!(
        stored_quote(&counter, SECOND).await.is_some(),
        "setup: it holds a quote"
    );

    let got = counter
        .ask(Some("FR1"), "get", json!({ "id": SECOND }))
        .await;
    got.assert_refused(400, ACCEPT_FIRST, "another branch's request");
    assert!(!got.text.contains("quoteId"), "{}", got.text);
    counter.finish().await;
}

/// §9 row 4: the rest of the answer is today's: the same fields, values and order, with only
/// `quoteId` added.
#[actix_web::test]
async fn the_rest_of_the_answer_is_unchanged() {
    let counter = open().await;
    held(&counter, "FR1", SECOND).await;
    let got = counter
        .ask(Some("FR1"), "get", json!({ "id": SECOND }))
        .await;
    assert_eq!(got.status, 200, "{}", got.text);
    let keys = keys_in_order(&got.text);
    assert!(
        keys.iter().any(|k| k == "quoteId"),
        "`quoteId` is added: {}",
        got.text
    );
    let others: Vec<&str> = keys
        .iter()
        .map(String::as_str)
        .filter(|k| *k != "quoteId")
        .collect();
    assert_eq!(others, TODAYS_KEYS, "today's keys, in today's order");
    let answer = got.json();
    assert_eq!(
        (
            &answer["paymentMethod"],
            &answer["type"],
            &answer["currency"],
            &answer["rate"],
            &answer["foreign"],
            &answer["thb"],
            &answer["inputAmount"],
            &answer["branchName"],
            &answer["id"],
            &answer["queue"],
        ),
        (
            &json!("Cash"),
            &json!("BUY"),
            &json!("USD"),
            &json!(32.1),
            &json!(100),
            &json!(3210),
            &json!(100),
            &json!("XC-B"),
            &json!(SECOND),
            &json!("K-00000000"),
        ),
        "today's values (`kiosk_http_test.rs` `the_answers_keep_v2s_key_order`): {}",
        got.text
    );
    counter.finish().await;
}
