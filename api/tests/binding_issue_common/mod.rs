//! The harness of `binding_issue_test.rs` (BQ-3, design `docs/design/blueprint/binding-quote.md`), on top of
//! TX-4's `Desk` (MAIN and BR2 publishing USD at 32.10 / 32.45, the DEMO customer `CU-TX4`).
//! Written by the sub-controller claude/644b3958 on `feat/binding-quote` before the implementation. **Do not edit,
//! delete or weaken**; add helpers in a file of your own.
//!
//! Every `issue` runs as production does (the command seam's `BranchTx`): committed on `Ok` — an
//! `ApprovalRequired` answer included — and rolled back on `Err`.

#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_cash::domain::reservation::port::StockReservations;
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xchange::application::workflows::exchange::binding_issue::{
    BindingIssueOutcome, BindingIssueRequest, IssueBindingQuote, IssuedQuote,
};
use xchange::application::workflows::exchange::quote::{QuoteRequest, QuoteRequestLine};

use super::quote_common::*;
use super::tx4_common::*;

/// The binding lifetime these tests enter (D-10): ten minutes.
pub const TTL_SECONDS: i64 = 600;

/// A `Desk` with MAIN holding `stock` USD and, when `ttl` is `Some`, the organisation's binding lifetime entered.
/// `CUSTOMER` is verified until a year from now (D-17).
pub async fn desk(stock: &str, ttl: Option<i64>) -> Desk {
    let desk = Desk::start().await;
    usd_position(&desk, "MAIN", stock).await;
    if let Some(seconds) = ttl {
        set_ttl(&desk, seconds).await;
    }
    verify(
        &desk,
        CUSTOMER,
        "VERIFIED",
        Some(Utc::now() + Duration::days(365)),
    )
    .await;
    desk
}

/// `exchange_settings.binding_quote_ttl_seconds` of the first organisation (BQ-0's column).
pub async fn set_ttl(desk: &Desk, seconds: i64) {
    desk.db
        .exec(&format!(
            "UPDATE exchange_settings SET binding_quote_ttl_seconds = {seconds} \
             WHERE organization_id = '{ORG_1}'"
        ))
        .await;
    assert_eq!(
        desk.db
            .count(&format!(
                "SELECT count(*) FROM exchange_settings \
                 WHERE organization_id = '{ORG_1}' AND binding_quote_ttl_seconds = {seconds}"
            ))
            .await,
        1,
        "the first organisation has one exchange_settings row"
    );
}

/// The branch's USD position at `quantity`.
pub async fn usd_position(desk: &Desk, branch: &str, quantity: &str) {
    desk.db
        .exec(&format!(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
             VALUES ('{ORG_1}', '{branch}', 'USD', {quantity}, {quantity} * 32) \
             ON CONFLICT (organization_id, branch_id, currency) DO UPDATE SET quantity = EXCLUDED.quantity"
        ))
        .await;
}

/// A DEMO customer of the first organisation.
pub async fn customer(desk: &Desk, id: &str) {
    desk.db
        .exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('{id}', '{ORG_1}', '{id}', 'Test Customer', 'DEMO', now())"
        ))
        .await;
}

/// The customer's latest verification case: `status`, `expires_at` (`Readiness::of` reads only these two).
pub async fn verify(
    desk: &Desk,
    customer_id: &str,
    status: &str,
    expires_at: Option<DateTime<Utc>>,
) {
    let reviewer = Uuid::new_v4();
    desk.db
        .exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{reviewer}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'test')",
            unique("reviewer")
        ))
        .await;
    let expires = expires_at.map_or("NULL".to_owned(), |t| format!("'{}'", t.to_rfc3339()));
    let reason = if matches!(status, "REJECTED" | "NEEDS_UPDATE" | "REVOKED") {
        "'OTHER'"
    } else {
        "NULL"
    };
    desk.db
        .exec(&format!(
            "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, policy_version, \
                 version, submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, created_at) \
             VALUES ('{}', '{ORG_1}'::uuid, '{customer_id}', 'STAFF', '{status}', 1, 1, now(), now(), {expires}, \
                 '{reviewer}', {reason}, clock_timestamp())",
            Uuid::new_v4()
        ))
        .await;
}

/// A DESK binding-quote request of one USD line at MAIN for `CUSTOMER`, paid in cash.
pub fn request(side: Side, quantity: &str) -> BindingIssueRequest {
    BindingIssueRequest {
        quote: QuoteRequest {
            branch_id: BranchId::new("MAIN"),
            customer_id: Some(CUSTOMER.to_owned()),
            channel: QuoteChannel::Desk,
            lines: vec![QuoteRequestLine {
                currency: CurrencyCode::parse("USD").unwrap(),
                shop_side: side,
                quantity: ForeignAmount::from_decimal(quantity.parse::<Decimal>().unwrap()),
                fee_thb: None,
                denomination_group: xc_rate::domain::currency::denomination_groups::GroupCode::all(
                ),
            }],
            payment_method: Some(PaymentMethod::Cash),
        },
        coupon_code: None,
        approval_request_id: None,
        replaces: None,
    }
}

/// `IssueBindingQuote::run_in` as the teller: committed on `Ok`, rolled back on `Err`.
pub async fn issue(
    desk: &Desk,
    req: BindingIssueRequest,
    now: DateTime<Utc>,
) -> AppResult<BindingIssueOutcome> {
    let service = IssueBindingQuote::new(desk.db.pool.clone(), desk.approvals.clone());
    let mut tx = desk.db.pool.begin().await.unwrap();
    let got = service.run_in(&mut tx, req, &teller(), now).await;
    if got.is_ok() {
        tx.commit().await.unwrap();
    } else {
        tx.rollback().await.unwrap();
    }
    got
}

/// The issued quote of an answer that must be `Issued`.
pub fn issued(got: AppResult<BindingIssueOutcome>) -> IssuedQuote {
    match got {
        Ok(BindingIssueOutcome::Issued(q)) => *q,
        other => panic!("expected Ok(Issued(_)), got {other:?}"),
    }
}

/// The request id of an answer that must be `ApprovalRequired`.
pub fn approval_required(got: AppResult<BindingIssueOutcome>) -> String {
    match got {
        Ok(BindingIssueOutcome::ApprovalRequired(id)) => id,
        other => panic!("expected Ok(ApprovalRequired(_)), got {other:?}"),
    }
}

/// The whole message of a refusal (`Err(AppError::Domain(m))`).
pub fn refusal<T: std::fmt::Debug>(got: AppResult<T>) -> String {
    match got {
        Err(pos_error::AppError::Domain(m)) => m,
        other => panic!("expected Err(AppError::Domain(_)), got {other:?}"),
    }
}

/// Rows of `quotes`.
pub async fn quotes(desk: &Desk) -> i64 {
    desk.db.count("SELECT count(*) FROM quotes").await
}

/// Rows of `stock_reservations`.
pub async fn reservations(desk: &Desk) -> i64 {
    desk.db
        .count("SELECT count(*) FROM stock_reservations")
        .await
}

/// `status|released_reason|quantity|expires_at = the quote's` of the quote's reservation rows, oldest first.
pub async fn reservation_rows(desk: &Desk, quote_id: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT r.status || '|' || COALESCE(r.released_reason, '-') || '|' || r.quantity::numeric(20,0)::text \
                || '|' || (r.expires_at = q.expires_at)::text \
           FROM stock_reservations r \
           JOIN quote_lines l ON l.id = r.quote_line_id \
           JOIN quotes q ON q.id = l.quote_id \
          WHERE q.id = $1 ORDER BY r.created_at, r.id",
    )
    .bind(quote_id)
    .fetch_all(&desk.db.pool)
    .await
    .expect("read stock_reservations")
}

/// `type|status|channel` of the stored quote.
pub async fn quote_row(desk: &Desk, quote_id: Uuid) -> String {
    desk.db
        .text(&format!(
            "SELECT type || '|' || status || '|' || channel FROM quotes WHERE id = '{quote_id}'"
        ))
        .await
        .unwrap_or_else(|| panic!("quote {quote_id} is a row"))
}

/// `type|object_id|branch_id|requested_by|status` of every `BINDING_QUOTE` request.
pub async fn binding_requests(desk: &Desk) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT type || '|' || object_id || '|' || COALESCE(branch_id, '') || '|' || requested_by || '|' || status \
           FROM approval_requests WHERE type = 'BINDING_QUOTE' ORDER BY requested_at, id",
    )
    .fetch_all(&desk.db.pool)
    .await
    .expect("read approval_requests")
}

/// MAIN's USD `reserved` figure at `at` (B05's `availability_in`, D-1/D-3).
pub async fn reserved_at(desk: &Desk, at: DateTime<Utc>) -> Decimal {
    let mut conn = desk.db.pool.acquire().await.unwrap();
    PgStockReservations::new()
        .availability_in(
            &mut conn,
            ORG_1.parse().unwrap(),
            &BranchId::new("MAIN"),
            CurrencyCode::parse("USD").unwrap(),
            at,
        )
        .await
        .unwrap_or_else(|e| panic!("availability_in: {e}"))
        .reserved
}

pub fn qty(text: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(text.parse::<Decimal>().unwrap())
}
