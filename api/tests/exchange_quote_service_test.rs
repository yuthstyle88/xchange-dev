//! Ticket #669 (TX-1+TX-3), Part 1 (TX-1) — `QuoteService::quote_in` against a real PostgreSQL: the
//! figures it computes, the row it stores, the organisation's quote lifetime, the derived `EXPIRED`,
//! and every refusal of ticket §8/§9.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §0 T1 (300 s by default, set per
//! organisation), §2 D-5 (only ACTIVE / ACCEPTED / CANCELLED are stored; `is_valid(now, current)` =
//! `now < expires_at` and the publication is the branch's current one, whatever the stored status;
//! `EXPIRED` is how an ACTIVE quote that fails it is shown), §3.1 (the stored row: `base_rate` = the
//! published rate, `final_rate` = `base_rate` until #560, `thb_amount` =
//! `ExchangeArithmetic::thb(quantity, final_rate, unit_size)`, `settle_thb` BUY `thb − fee`, SELL
//! `thb + fee`, one line only), §3.6 (`exchange_settings.quote_ttl_seconds`), §4.3 (steps 1–6:
//! validate, `PublishedRates::line_in`, `StalePolicy::admits`, figures, `expires_at = now + ttl`,
//! write), §7 (`QuoteService::quote_in` [required]), §9.1 TX-1. Rates design
//! `docs/design/blueprint/rates.md` §3.2 (HALF_UP to 2 places once, at the end) and §4.8
//! (`line_in`, stale at read time, `StalePolicy`: STOP refuses, LIMIT refuses above
//! `stale_limit_thb`, NULL admits).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. `quote_in` runs in a transaction the test opens. A refused call is followed by a COMMIT of
//! that transaction and the tables are counted: §4.3 writes last, so a refusal leaves nothing even
//! when the caller commits. `now` is passed in whole seconds; staleness is decided by `line_in` on
//! the wall clock (a reference fetched 30 h ago against 24 h), so no test depends on timing.
//!
//! Signatures this test assumes ([required] in ticket §7 unless marked; "choice" marks a form the
//! design leaves open, which the implementer follows):
//! - `xchange::application::workflows::exchange::quote::{QuoteService, QuoteRequest, QuoteRequestLine}`.
//!   - choice: `QuoteService::new(pool: sqlx::PgPool) -> QuoteService`, wiring `PgPublishedRates`,
//!     `PgQuoteRepository` and `PgSettingsRepository` itself.
//!   - `QuoteService::quote_in(&self, tx: &mut sqlx::Transaction<'_, Postgres>, req: QuoteRequest,
//!     who: &SaveActor, now: DateTime<Utc>) -> AppResult<Quote>` (async).
//!   - choice: `QuoteRequest { branch_id: BranchId, customer_id: Option<String>, channel:
//!     QuoteChannel, lines: Vec<QuoteRequestLine> }` and `QuoteRequestLine { currency: CurrencyCode,
//!     shop_side: Side, quantity: ForeignAmount, fee_thb: Option<Money> }`, all fields public
//!     (ticket §8's fields).
//! - `xc_exchange::domain::quote::quote::{Quote, QuoteStatus, QuoteChannel}`:
//!   - `QuoteStatus { Active, Accepted, Cancelled }` [required];
//!   - choice: `QuoteChannel { Desk, Kiosk, Booking, App }`, stored as `DESK` … `APP`;
//!   - choice: `Quote: Clone` with public fields `id` (its `Display` is the row's uuid) and
//!     `status: QuoteStatus`;
//!   - `Quote::is_valid(&self, now: DateTime<Utc>, current_publication: Uuid) -> bool` and
//!     `Quote::status_at(&self, now: DateTime<Utc>, current_publication: Uuid)`, whose answer has
//!     `as_str()` giving `"ACTIVE"`, `"ACCEPTED"`, `"CANCELLED"` or `"EXPIRED"` (choice: the
//!     publication is a plain `Uuid`, since `xc-exchange` depends on kernel crates only and cannot
//!     name `xc_rate`'s `PublicationId`).
//! - The stored columns of §3.1 (`exchange_quotes_migration_test.rs`).
//! - Which `AppError` variant each refusal is (`validationFailed`, `rateUnavailable`) is not pinned
//!   here; `v3_quotes_route_test.rs` pins the v3 codes the command answers.
//! - The stale policy `quote_in` applies is **the policy version the line was published under**:
//!   `published_rate_lines.source_snapshot.policy_version_id`, and that row's `on_stale` and
//!   `stale_limit_thb` (rates §4.8, *"of the policy version it was published under"*; ticket
//!   author's ruling 6 on #669, 2026-10-09). A policy saved after the publication does not change
//!   how its lines are quoted. A MANUAL line with no reference is never stale.
//! - An organisation with no `exchange_settings` row is quoted with 300 s, the migration's default,
//!   and the quote is stored as usual (ruling 7).

mod quote_common;

use chrono::{DateTime, Duration, Utc};
use pos_error::AppResult;
use quote_common::*;
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel, QuoteStatus};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Side};
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;

const TELLER: &str = "employee-7";

fn teller() -> SaveActor {
    SaveActor {
        id: TELLER.to_owned(),
        username: format!("user-{TELLER}"),
        employee_id: Some(TELLER.to_owned()),
        standing: StandingKind::Staff(AccessRole::Teller),
    }
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a known currency")
}

fn money(text: &str) -> Money {
    Money::parse(text).expect("an amount")
}

/// One line; the quantity is taken as given (the service checks it against the minor unit).
fn line(currency: &str, side: Side, quantity: &str, fee: Option<&str>) -> QuoteRequestLine {
    QuoteRequestLine {
        currency: code(currency),
        shop_side: side,
        quantity: ForeignAmount::from_decimal(dec(quantity)),
        denomination_group: xc_rate::domain::currency::denomination_groups::GroupCode::all(),
        fee_thb: fee.map(money),
    }
}

fn request(branch: &str, lines: Vec<QuoteRequestLine>) -> QuoteRequest {
    QuoteRequest {
        branch_id: BranchId::new(branch),
        customer_id: None,
        channel: QuoteChannel::Desk,
        lines,
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

async fn quoted(db: &TestDb, req: QuoteRequest, now: DateTime<Utc>) -> Quote {
    quote(db, req, now)
        .await
        .unwrap_or_else(|e| panic!("quote_in refused: {e}"))
}

/// The stored quote line of `q`, as `quote_lines` holds it.
#[derive(Debug, PartialEq, sqlx::FromRow)]
struct StoredLine {
    published_rate_line_id: String,
    shop_side: String,
    currency: String,
    quantity: Decimal,
    unit_size: Decimal,
    base_rate: Decimal,
    final_rate: Decimal,
    thb_amount: String,
    fee_thb: String,
    settle_thb: String,
    line_no: i32,
}

async fn stored_lines(db: &TestDb, q: &Quote) -> Vec<StoredLine> {
    sqlx::query_as::<_, StoredLine>(
        "SELECT published_rate_line_id::text AS published_rate_line_id, shop_side, \
                currency::text AS currency, quantity, unit_size, base_rate, final_rate, \
                thb_amount::text AS thb_amount, fee_thb::text AS fee_thb, \
                settle_thb::text AS settle_thb, line_no \
         FROM quote_lines WHERE quote_id = $1::uuid ORDER BY line_no",
    )
    .bind(q.id.to_string())
    .fetch_all(&db.pool)
    .await
    .expect("read quote_lines")
}

/// The one stored line of `q`.
async fn stored_line(db: &TestDb, q: &Quote) -> StoredLine {
    let mut lines = stored_lines(db, q).await;
    assert_eq!(lines.len(), 1, "one line (§3.1): {lines:?}");
    lines.remove(0)
}

/// (organization_id, branch_id, customer_id, type, channel, status, publication_id, expires_at,
/// created_by, accepted_by_transaction_id) of the stored quote `q`.
type StoredQuote = (
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    DateTime<Utc>,
    String,
    Option<String>,
);

async fn stored_quote(db: &TestDb, q: &Quote) -> StoredQuote {
    sqlx::query_as(
        "SELECT organization_id::text, branch_id, customer_id, type, channel, status, \
                publication_id::text, expires_at, created_by, accepted_by_transaction_id::text \
         FROM quotes WHERE id = $1::uuid",
    )
    .bind(q.id.to_string())
    .fetch_optional(&db.pool)
    .await
    .expect("read quotes")
    .unwrap_or_else(|| panic!("the returned quote {} is a row of quotes", q.id))
}

fn uuid(text: &str) -> Uuid {
    Uuid::parse_str(text).expect("a uuid")
}

// ------------------------------------------------------------ figures

#[tokio::test]
async fn jpy_1000_at_23_5_per_100_is_235_00_and_the_quote_is_stored_as_priced() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[JPY, USD]).await;
    let now = whole_second_now();

    let q = quoted(
        &db,
        request("MAIN", vec![line("JPY", Side::Buy, "1000", None)]),
        now,
    )
    .await;

    assert_eq!(
        stored_line(&db, &q).await,
        StoredLine {
            published_rate_line_id: p.line("JPY").to_owned(),
            shop_side: "BUY".into(),
            currency: "JPY".into(),
            quantity: dec("1000"),
            unit_size: dec("100"),
            base_rate: dec("23.5"),
            final_rate: dec("23.5"),
            thb_amount: "235.00".into(),
            fee_thb: "0.00".into(),
            settle_thb: "235.00".into(),
            line_no: 1,
        },
        "JPY 1,000 at 23.5 per 100 (§9.1); no fee ⇒ fee 0.00, settle = thb"
    );
    assert_eq!(
        stored_quote(&db, &q).await,
        (
            ORG_1.to_owned(),
            "MAIN".to_owned(),
            None,
            "INDICATIVE".to_owned(),
            "DESK".to_owned(),
            "ACTIVE".to_owned(),
            p.id.clone(),
            now + Duration::seconds(300),
            TELLER.to_owned(),
            None,
        ),
        "the stored quote: the branch's organisation, the current publication, the caller"
    );
    assert_eq!(
        q.status,
        QuoteStatus::Active,
        "the returned quote is ACTIVE"
    );
    // the line belongs to the publication the quote names
    assert_eq!(
        db.text(&format!(
            "SELECT l.publication_id::text FROM published_rate_lines l \
             JOIN quote_lines ql ON ql.published_rate_line_id = l.id \
             WHERE ql.quote_id = '{}'",
            q.id
        ))
        .await,
        Some(p.id.clone()),
        "the quote's line is a line of its publication"
    );

    // the shop's other side reads the other rate: SELL JPY 1,000 at 24.0 per 100
    let q = quoted(
        &db,
        request("MAIN", vec![line("JPY", Side::Sell, "1000", None)]),
        now,
    )
    .await;
    let l = stored_line(&db, &q).await;
    assert_eq!(
        (l.shop_side.as_str(), l.base_rate, l.thb_amount.as_str()),
        ("SELL", dec("24.0"), "240.00"),
        "SELL uses the sell rate"
    );
    assert_eq!(db.quote_rows().await, (2, 2), "two quotes, one line each");
    db.finish().await;
}

#[tokio::test]
async fn buy_with_a_fee_settles_thb_minus_fee_and_sell_settles_thb_plus_fee() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    let now = whole_second_now();

    let buy = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "100", Some("50.00"))]),
        now,
    )
    .await;
    let l = stored_line(&db, &buy).await;
    assert_eq!(
        (
            l.thb_amount.as_str(),
            l.fee_thb.as_str(),
            l.settle_thb.as_str()
        ),
        ("3210.00", "50.00", "3160.00"),
        "BUY USD 100 at 32.10, fee 50: the shop pays thb − fee"
    );

    let sell = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Sell, "100", Some("50.00"))]),
        now,
    )
    .await;
    let l = stored_line(&db, &sell).await;
    assert_eq!(
        (
            l.thb_amount.as_str(),
            l.fee_thb.as_str(),
            l.settle_thb.as_str()
        ),
        ("3245.00", "50.00", "3295.00"),
        "SELL USD 100 at 32.45, fee 50: the shop takes thb + fee"
    );

    // a fee equal to the THB amount is admitted: BUY settles 0.00
    let all = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "1", Some("32.10"))]),
        now,
    )
    .await;
    assert_eq!(
        stored_line(&db, &all).await.settle_thb,
        "0.00",
        "fee = thb_amount"
    );
    db.finish().await;
}

#[tokio::test]
async fn thb_is_rounded_half_up_once_where_bankers_rounding_would_round_down() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD, Line::manual("JPY", "100", "24.5", "25.0")])
        .await;
    let now = whole_second_now();

    // 1.05 × 32.10 = 33.705: HALF_UP 33.71, banker's 33.70
    let q = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "1.05", None)]),
        now,
    )
    .await;
    assert_eq!(
        stored_line(&db, &q).await.thb_amount,
        "33.71",
        "USD 1.05 at 32.10"
    );

    // 1 × 24.5 ÷ 100 = 0.245: HALF_UP 0.25, banker's 0.24 — the division by unit_size is inside
    // the one rounding
    let q = quoted(
        &db,
        request("MAIN", vec![line("JPY", Side::Buy, "1", None)]),
        now,
    )
    .await;
    let l = stored_line(&db, &q).await;
    assert_eq!(
        (l.thb_amount.as_str(), l.settle_thb.as_str()),
        ("0.25", "0.25"),
        "JPY 1 at 24.5 per 100"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_figures_come_from_the_current_publication() {
    let db = TestDb::start().await;
    let first = db.publish("MAIN", &[JPY]).await;
    let now = whole_second_now();
    let q1 = quoted(
        &db,
        request("MAIN", vec![line("JPY", Side::Buy, "1000", None)]),
        now,
    )
    .await;

    let second = db
        .publish("MAIN", &[Line::manual("JPY", "100", "22.0", "24.0")])
        .await;
    let q2 = quoted(
        &db,
        request("MAIN", vec![line("JPY", Side::Buy, "1000", None)]),
        now,
    )
    .await;

    let (l1, l2) = (stored_line(&db, &q1).await, stored_line(&db, &q2).await);
    assert_eq!(
        (l1.published_rate_line_id.as_str(), l1.thb_amount.as_str()),
        (first.line("JPY"), "235.00"),
        "the first quote keeps the first publication's line"
    );
    assert_eq!(
        (l2.published_rate_line_id.as_str(), l2.thb_amount.as_str()),
        (second.line("JPY"), "220.00"),
        "the second quote is priced on the new publication"
    );
    assert_eq!(stored_quote(&db, &q2).await.6, second.id, "publication_id");
    db.finish().await;
}

#[tokio::test]
async fn the_channel_and_the_customer_are_stored_as_asked() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    db.customer("C-1", "MAIN").await;
    let now = whole_second_now();

    let mut req = request("MAIN", vec![line("USD", Side::Sell, "10", None)]);
    req.customer_id = Some("C-1".to_owned());
    req.channel = QuoteChannel::Kiosk;
    let q = quoted(&db, req, now).await;
    let s = stored_quote(&db, &q).await;
    assert_eq!(
        (s.2.as_deref(), s.4.as_str()),
        (Some("C-1"), "KIOSK"),
        "customer_id and channel"
    );

    for (channel, text) in [
        (QuoteChannel::Desk, "DESK"),
        (QuoteChannel::Booking, "BOOKING"),
        (QuoteChannel::App, "APP"),
    ] {
        let mut req = request("MAIN", vec![line("USD", Side::Sell, "10", None)]);
        req.channel = channel;
        let q = quoted(&db, req, now).await;
        assert_eq!(stored_quote(&db, &q).await.4, text, "channel {text}");
    }
    db.finish().await;
}

// ------------------------------------------------------------ lifetime (T1, D-5)

#[tokio::test]
async fn expires_at_is_now_plus_the_organisations_quote_ttl() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    let org2 = "00000000-0000-0000-0000-000000000002";
    db.second_organisation(org2, "XC2", "B2").await;
    db.exec(&format!(
        "INSERT INTO exchange_settings (organization_id, quote_ttl_seconds, updated_by, updated_at) \
         VALUES ('{org2}', 600, 'test', now())"
    ))
    .await;
    db.publish("B2", &[USD]).await;
    let now = whole_second_now();
    let usd = || vec![line("USD", Side::Buy, "10", None)];

    // the default row: 300 s
    let q = quoted(&db, request("MAIN", usd()), now).await;
    assert_eq!(
        stored_quote(&db, &q).await.7,
        now + Duration::seconds(300),
        "the migration's 300"
    );

    // ADMIN changes organisation 1's TTL: the next quote reads it
    db.exec(&format!(
        "UPDATE exchange_settings SET quote_ttl_seconds = 90 WHERE organization_id = '{ORG_1}'"
    ))
    .await;
    let q = quoted(&db, request("MAIN", usd()), now).await;
    assert_eq!(
        stored_quote(&db, &q).await.7,
        now + Duration::seconds(90),
        "organisation 1 at 90 s"
    );

    // organisation 2's branch reads organisation 2's row
    let q = quoted(&db, request("B2", usd()), now).await;
    let s = stored_quote(&db, &q).await;
    assert_eq!(
        (s.0.as_str(), s.7),
        (org2, now + Duration::seconds(600)),
        "organisation 2 at 600 s"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_quote_shows_expired_from_expires_at_on() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    let now = whole_second_now();
    let q = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "10", None)]),
        now,
    )
    .await;
    let current = uuid(&p.id);
    let expires = now + Duration::seconds(300);

    for (at, valid, shown) in [
        (now, true, "ACTIVE"),
        (expires - Duration::seconds(1), true, "ACTIVE"),
        (expires, false, "EXPIRED"),
        (expires + Duration::seconds(1), false, "EXPIRED"),
    ] {
        assert_eq!(q.is_valid(at, current), valid, "is_valid at {at}");
        assert_eq!(q.status_at(at, current).as_str(), shown, "status_at {at}");
    }
    // nothing is written when a quote expires (D-5: derived)
    assert_eq!(
        stored_quote(&db, &q).await.5,
        "ACTIVE",
        "the stored status stays ACTIVE"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_new_publication_expires_the_quote_before_expires_at() {
    let db = TestDb::start().await;
    let first = db.publish("MAIN", &[USD]).await;
    let now = whole_second_now();
    let q = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "10", None)]),
        now,
    )
    .await;
    assert!(
        q.is_valid(now, uuid(&first.id)),
        "valid on its own publication"
    );

    db.publish("MAIN", &[USD]).await;
    let current = uuid(&db.current_publication("MAIN").await);
    assert_ne!(current, uuid(&first.id), "the setup: a new publication");
    assert!(
        !q.is_valid(now, current),
        "a new publication, before expires_at: not valid"
    );
    assert_eq!(q.status_at(now, current).as_str(), "EXPIRED");
    db.finish().await;
}

#[tokio::test]
async fn validity_ignores_the_stored_status_and_only_active_is_shown_expired() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[USD]).await;
    let now = whole_second_now();
    let q = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "10", None)]),
        now,
    )
    .await;
    let current = uuid(&p.id);
    let other = Uuid::new_v4();
    let late = now + Duration::seconds(301);

    let mut accepted = q.clone();
    accepted.status = QuoteStatus::Accepted;
    assert!(
        accepted.is_valid(now, current),
        "ACCEPTED, in time, current"
    );
    assert!(
        !accepted.is_valid(late, current),
        "ACCEPTED past expires_at: not valid (a late approval, D-5)"
    );
    assert!(
        !accepted.is_valid(now, other),
        "ACCEPTED after a new publication: not valid"
    );
    assert_eq!(
        accepted.status_at(late, current).as_str(),
        "ACCEPTED",
        "an ACCEPTED quote is shown ACCEPTED"
    );

    let mut cancelled = q.clone();
    cancelled.status = QuoteStatus::Cancelled;
    assert_eq!(
        cancelled.status_at(late, current).as_str(),
        "CANCELLED",
        "a CANCELLED quote is shown CANCELLED"
    );
    assert_eq!(q.status_at(now, current).as_str(), "ACTIVE");
    assert_eq!(
        q.status_at(now, other).as_str(),
        "EXPIRED",
        "ACTIVE on an old publication"
    );
    db.finish().await;
}

// ------------------------------------------------------------ refusals

/// `req` is refused and nothing is stored, even though the transaction is committed.
async fn refused(db: &TestDb, req: QuoteRequest, what: &str) {
    let before = db.quote_rows().await;
    let answer = quote(db, req, whole_second_now()).await;
    assert!(
        answer.is_err(),
        "{what}: refused, got {:?}",
        answer.map(|q| q.id.to_string())
    );
    assert_eq!(db.quote_rows().await, before, "{what}: nothing stored");
}

#[tokio::test]
async fn more_or_fewer_than_one_line_is_refused() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[JPY, USD]).await;
    refused(
        &db,
        request(
            "MAIN",
            vec![
                line("USD", Side::Buy, "10", None),
                line("JPY", Side::Buy, "1000", None),
            ],
        ),
        "two lines",
    )
    .await;
    refused(&db, request("MAIN", vec![]), "no line").await;
    // the same line alone is quoted
    quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "10", None)]),
        whole_second_now(),
    )
    .await;
    assert_eq!(db.quote_rows().await, (1, 1));
    db.finish().await;
}

#[tokio::test]
async fn a_quantity_that_does_not_fit_the_minor_unit_is_refused() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[JPY, USD]).await;
    refused(
        &db,
        request("MAIN", vec![line("JPY", Side::Buy, "1000.5", None)]),
        "JPY has no minor unit",
    )
    .await;
    refused(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "10.001", None)]),
        "USD has 2 places",
    )
    .await;
    refused(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "0", None)]),
        "zero",
    )
    .await;
    // trailing zeros within the minor unit are fine
    let q = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "10.50", None)]),
        whole_second_now(),
    )
    .await;
    assert_eq!(
        stored_line(&db, &q).await.thb_amount,
        "337.05",
        "10.50 × 32.10"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_fee_above_the_thb_amount_is_refused_on_either_side() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    refused(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "1", Some("32.11"))]),
        "BUY, fee 32.11 > 32.10",
    )
    .await;
    refused(
        &db,
        request("MAIN", vec![line("USD", Side::Sell, "1", Some("32.46"))]),
        "SELL, fee 32.46 > 32.45",
    )
    .await;
    db.finish().await;
}

#[tokio::test]
async fn no_current_line_for_the_branch_currency_and_side_is_refused() {
    let db = TestDb::start().await;
    db.branch("B-EMPTY").await;
    refused(
        &db,
        request("B-EMPTY", vec![line("USD", Side::Buy, "10", None)]),
        "a branch with no publication",
    )
    .await;

    db.publish(
        "MAIN",
        &[
            USD,
            Line {
                code: "GBP",
                unit_size: "1",
                buy: Some("40.00"),
                sell: None,
                stale_under: None,
            },
        ],
    )
    .await;
    refused(
        &db,
        request("MAIN", vec![line("EUR", Side::Buy, "10", None)]),
        "EUR is not published",
    )
    .await;
    refused(
        &db,
        request("MAIN", vec![line("GBP", Side::Sell, "10", None)]),
        "GBP has no sell rate",
    )
    .await;
    // the published side is quoted
    let q = quoted(
        &db,
        request("MAIN", vec![line("GBP", Side::Buy, "10", None)]),
        whole_second_now(),
    )
    .await;
    assert_eq!(stored_line(&db, &q).await.thb_amount, "400.00", "GBP BUY");
    db.finish().await;
}

#[tokio::test]
async fn a_stale_line_under_stop_is_refused() {
    let db = TestDb::start().await;
    let stop = db.policy("MAIN", Some("STOP"), None).await;
    db.publish(
        "MAIN",
        &[
            Line {
                stale_under: Some(stop),
                ..USD
            },
            JPY,
        ],
    )
    .await;
    refused(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "1", None)]),
        "USD is stale under STOP",
    )
    .await;
    // STOP refuses only a stale line: JPY (MANUAL, never stale) is quoted
    quoted(
        &db,
        request("MAIN", vec![line("JPY", Side::Buy, "1000", None)]),
        whole_second_now(),
    )
    .await;
    assert_eq!(db.quote_rows().await, (1, 1));
    db.finish().await;
}

#[tokio::test]
async fn a_stale_line_under_limit_is_refused_only_above_the_limit_and_null_admits() {
    let db = TestDb::start().await;
    let limit = db.policy("MAIN", Some("LIMIT"), Some("1000.00")).await;
    db.publish(
        "MAIN",
        &[Line {
            stale_under: Some(limit),
            ..USD
        }],
    )
    .await;
    // 31 × 32.10 = 995.10 ≤ 1,000.00
    let q = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "31", None)]),
        whole_second_now(),
    )
    .await;
    assert_eq!(stored_line(&db, &q).await.thb_amount, "995.10");
    // 32 × 32.10 = 1,027.20 > 1,000.00
    refused(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "32", None)]),
        "stale, above LIMIT",
    )
    .await;

    // B-NULL: the same stale line under a policy with no on_stale is quoted at any amount
    db.branch("B-NULL").await;
    let none = db.policy("B-NULL", None, None).await;
    db.publish(
        "B-NULL",
        &[Line {
            stale_under: Some(none),
            ..USD
        }],
    )
    .await;
    quoted(
        &db,
        request("B-NULL", vec![line("USD", Side::Buy, "1000", None)]),
        whole_second_now(),
    )
    .await;
    assert_eq!(db.quote_rows().await, (2, 2));
    db.finish().await;
}

#[tokio::test]
async fn the_policy_the_line_was_published_under_governs_not_a_later_one() {
    let db = TestDb::start().await;
    let now = whole_second_now();

    // MAIN: published under v1 (STOP); v2 (no on_stale, no age limit) is saved afterwards
    let v1 = db.policy("MAIN", Some("STOP"), None).await;
    db.publish(
        "MAIN",
        &[
            Line {
                stale_under: Some(v1.clone()),
                ..USD
            },
            JPY,
        ],
    )
    .await;
    db.exec(&format!(
        "UPDATE rate_policy_versions SET valid_to = now() WHERE id = '{v1}'"
    ))
    .await;
    let v2 = db.policy("MAIN", None, None).await;
    db.exec(&format!(
        "UPDATE rate_policy_versions SET max_reference_age_hours = NULL WHERE id = '{v2}'"
    ))
    .await;
    refused(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "1", None)]),
        "stale under v1's STOP although v2 is in force",
    )
    .await;
    // the MANUAL line of the same publication carries no policy and is never stale
    quoted(
        &db,
        request("MAIN", vec![line("JPY", Side::Buy, "1000", None)]),
        now,
    )
    .await;

    // B-LATE: published under v1 (no on_stale); v2 (STOP) is saved afterwards
    db.branch("B-LATE").await;
    let v1 = db.policy("B-LATE", None, None).await;
    db.publish(
        "B-LATE",
        &[Line {
            stale_under: Some(v1.clone()),
            ..USD
        }],
    )
    .await;
    db.exec(&format!(
        "UPDATE rate_policy_versions SET valid_to = now() WHERE id = '{v1}'"
    ))
    .await;
    db.policy("B-LATE", Some("STOP"), None).await;
    let q = quoted(
        &db,
        request("B-LATE", vec![line("USD", Side::Buy, "10", None)]),
        now,
    )
    .await;
    assert_eq!(
        stored_line(&db, &q).await.thb_amount,
        "321.00",
        "stale, admitted under v1 although v2 (STOP) is in force"
    );
    assert_eq!(db.quote_rows().await, (2, 2));
    db.finish().await;
}

#[tokio::test]
async fn no_settings_row_for_the_organisation_means_300_seconds() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    db.exec(&format!(
        "DELETE FROM exchange_settings WHERE organization_id = '{ORG_1}'"
    ))
    .await;
    let now = whole_second_now();
    let q = quoted(
        &db,
        request("MAIN", vec![line("USD", Side::Buy, "10", None)]),
        now,
    )
    .await;
    let s = stored_quote(&db, &q).await;
    assert_eq!(
        (s.5.as_str(), s.7),
        ("ACTIVE", now + Duration::seconds(300)),
        "stored, expiring at now + 300 s"
    );
    assert_eq!(db.quote_rows().await, (1, 1));
    db.finish().await;
}
