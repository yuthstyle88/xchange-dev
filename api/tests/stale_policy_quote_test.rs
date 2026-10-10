//! Ticket #769 — the desk quote judges a stale line by the policy it was published under, read
//! through `PublishedRates::stale_policy_in`; a stale line whose policy cannot be found is an
//! error and writes nothing.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.8 "Stale at read time" (the policy version the
//! line was published under; `StalePolicy::admits`: STOP refuses, LIMIT refuses a THB amount
//! above `stale_limit_thb`, NULL admits), §4.9 R5 (*a stale line whose policy cannot be found must
//! be an error*). Ticket #769 §7 (the quote's refusal stays `rateUnavailable`), §9 (edge cases).
//! Quote design `docs/design/blueprint/quote-transaction.md` §4.3 (step 3 is the stale check;
//! step 6 writes, so an error before it writes nothing).
//!
//! The reader is `QuoteService::quote_in` (`exchange_quote_service_test.rs` drives it the same
//! way): it runs in a transaction the test opens and **commits whatever the answer**, then the
//! tables are counted, so "writes nothing" holds even when the caller commits. Stale lines are
//! seeded with SQL by `stale_policy_769_common` (why: see its header).
//!
//! Already pinned before this ticket, not repeated here (`exchange_quote_service_test.rs`):
//! `a_stale_line_under_stop_is_refused` (STOP refuses; that the refusal is not `Internal` is pinned
//! below), the B-NULL half of
//! `a_stale_line_under_limit_is_refused_only_above_the_limit_and_null_admits` (NULL admits),
//! `the_policy_the_line_was_published_under_governs_not_a_later_one`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips.
//!
//! Signatures this test assumes:
//! - [required] `PublishedRates::stale_policy_in(tx, line: &PublishedSide) ->
//!   AppResult<StalePolicy>` (ticket #769 §7), which `quote_in` calls for a stale line; this file
//!   pins its effect through `quote_in`, not the call;
//! - `QuoteService::new(PgPool)`, `QuoteService::quote_in(&self, tx, QuoteRequest, &SaveActor,
//!   DateTime<Utc>) -> AppResult<Quote>`, `QuoteRequest`, `QuoteRequestLine` as
//!   `exchange_quote_service_test.rs` names them (TX-1, merged);
//! - a stale line whose policy cannot be found answers `AppError::Internal` (#769 §8); a policy
//!   refusal answers an error that is not `Internal` (today `Domain("rateUnavailable: …")`).

mod quote_common;
mod stale_policy_769_common;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use quote_common::{TestDb, whole_second_now};
use rust_decimal::Decimal;
use stale_policy_769_common::{
    Names, id_of, policy, policy_rows, publish_auto, reference_age, unfindable,
};
use std::str::FromStr;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;

const MAIN: &str = "MAIN";
/// USD per 1, as `quote_common::USD`: BUY 32.10, SELL 32.45.
const USD: (&str, &str, &str, &str) = ("USD", "1", "32.10", "32.45");

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-7".to_owned(),
        username: "user-employee-7".to_owned(),
        employee_id: Some("employee-7".to_owned()),
        role: Role::Front,
    }
}

fn request(side: Side, quantity: &str) -> QuoteRequest {
    QuoteRequest {
        branch_id: BranchId::new(MAIN),
        customer_id: None,
        channel: QuoteChannel::Desk,
        lines: vec![QuoteRequestLine {
            currency: CurrencyCode::parse("USD").expect("USD"),
            shop_side: side,
            quantity: ForeignAmount::from_decimal(Decimal::from_str(quantity).expect("a quantity")),
            fee_thb: None,
        }],
    }
}

/// `quote_in` in a transaction of its own, committed whatever the answer.
async fn quote(db: &TestDb, req: QuoteRequest, now: DateTime<Utc>) -> AppResult<Quote> {
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let answer = service.quote_in(&mut tx, req, &teller(), now).await;
    tx.commit().await.expect("commit");
    answer
}

/// The one stored line of `q`: `(published_rate_line_id, thb_amount)` as text.
async fn stored(db: &TestDb, q: &Quote) -> (String, String) {
    sqlx::query_as(
        "SELECT published_rate_line_id::text, thb_amount::text FROM quote_lines \
         WHERE quote_id = $1::uuid",
    )
    .bind(q.id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("read the quote's line")
}

async fn quoted(db: &TestDb, req: QuoteRequest, what: &str) -> Quote {
    match quote(db, req, whole_second_now()).await {
        Ok(q) => q,
        Err(e) => panic!("{what}: quoted expected, got the error {e}"),
    }
}

/// A refusal by the policy: an error that is not `Internal`, and nothing stored.
async fn refused_by_policy(db: &TestDb, req: QuoteRequest, what: &str) {
    let before = db.quote_rows().await;
    match quote(db, req, whole_second_now()).await {
        Ok(q) => panic!("{what}: refused expected, got the quote {}", q.id),
        Err(AppError::Internal(m)) => {
            panic!("{what}: a policy refusal is the quote's existing answer, not Internal: {m}")
        }
        Err(_) => {}
    }
    assert_eq!(db.quote_rows().await, before, "{what}: nothing stored");
}

/// #769 §8–9: `Internal`, and nothing stored although the transaction commits.
async fn internal(db: &TestDb, req: QuoteRequest, what: &str) {
    let before = db.quote_rows().await;
    match quote(db, req, whole_second_now()).await {
        Err(AppError::Internal(_)) => {}
        Err(other) => panic!("{what}: AppError::Internal expected, got the error {other}"),
        Ok(q) => panic!(
            "{what}: AppError::Internal expected, but the stale price was quoted ({})",
            q.id
        ),
    }
    assert_eq!(db.quote_rows().await, before, "{what}: nothing stored");
}

/// §9 "Stale, snapshot without `policy_version_id`" and "Stale, policy row missing": the quote
/// answers `Internal` on either side and stores nothing (today it quotes the stale price). The
/// same line made fresh is quoted: no lookup for a line that is not stale (§9 "Line not stale").
#[tokio::test]
async fn a_stale_line_whose_policy_cannot_be_found_is_internal_and_stores_nothing() {
    let db = TestDb::start().await;
    for (names, label) in unfindable() {
        assert_eq!(policy_rows(&db.pool, &names).await, 0, "the setup: {label}");
        let ids = publish_auto(&db.pool, MAIN, &names, 30, &[USD]).await;
        internal(&db, request(Side::Buy, "1"), &format!("BUY, {label}")).await;
        internal(&db, request(Side::Sell, "1"), &format!("SELL, {label}")).await;

        let line = id_of(&ids, "USD");
        reference_age(&db.pool, &line, 1).await;
        let before = db.quote_rows().await;
        let q = quoted(&db, request(Side::Buy, "1"), &format!("fresh, {label}")).await;
        assert_eq!(stored(&db, &q).await, (line, "32.10".to_owned()), "{label}");
        assert_eq!(
            db.quote_rows().await,
            (before.0 + 1, before.1 + 1),
            "{label}"
        );
    }
    db.finish().await;
}

/// §9 "Stale, `LIMIT` at exactly the limit": 32 × 32.10 = 1,027.20 is quoted under a 1,027.20
/// limit and refused, with the quote's existing answer, under 1,027.19 (one satang over).
#[tokio::test]
async fn a_stale_line_under_limit_is_quoted_at_exactly_the_limit_and_refused_one_satang_over() {
    let db = TestDb::start().await;
    let at = policy(&db.pool, MAIN, Some("LIMIT"), Some("1027.20")).await;
    let ids = publish_auto(&db.pool, MAIN, &Names::Policy(at), 30, &[USD]).await;
    let q = quoted(
        &db,
        request(Side::Buy, "32"),
        "1,027.20 under a 1,027.20 limit",
    )
    .await;
    assert_eq!(
        stored(&db, &q).await,
        (id_of(&ids, "USD"), "1027.20".to_owned())
    );

    let below = policy(&db.pool, MAIN, Some("LIMIT"), Some("1027.19")).await;
    publish_auto(&db.pool, MAIN, &Names::Policy(below), 30, &[USD]).await;
    refused_by_policy(
        &db,
        request(Side::Buy, "32"),
        "1,027.20 under a 1,027.19 limit",
    )
    .await;
    assert_eq!(db.quote_rows().await, (1, 1));
    db.finish().await;
}

/// §9 "Stale, `STOP`": refused with the quote's existing answer, which is not `Internal`
/// (`exchange_quote_service_test.rs` pins only that STOP refuses).
#[tokio::test]
async fn a_stale_line_under_stop_is_refused_with_the_existing_answer_not_internal() {
    let db = TestDb::start().await;
    let stop = policy(&db.pool, MAIN, Some("STOP"), None).await;
    publish_auto(&db.pool, MAIN, &Names::Policy(stop), 30, &[USD]).await;
    refused_by_policy(&db, request(Side::Buy, "1"), "BUY under STOP").await;
    refused_by_policy(&db, request(Side::Sell, "1"), "SELL under STOP").await;
    assert_eq!(db.quote_rows().await, (0, 0));
    db.finish().await;
}
