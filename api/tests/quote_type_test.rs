//! Up-front tests for BQ-2 (design `docs/design/blueprint/binding-quote.md` D-1, D-2, D-10, §10): `QuoteService::
//! price_in` is the one priced path and writes the type and lifetime its `QuoteTerms` name. Written by the
//! sub-controller claude/644b3958 on `feat/binding-quote` before the implementation. **Do not edit, delete or
//! weaken.**
//!
//! - `QuoteTerms::Indicative` is today's quote: `INDICATIVE`, `expires_at = now + quote_ttl_seconds`.
//! - `QuoteTerms::Binding { ttl }` prices the same lines the same way and stores `BINDING` with
//!   `expires_at = now + ttl`; `quote_ttl_seconds` plays no part. `price_in` checks no binding rule (channel,
//!   customer, limit, stock): those are `IssueBindingQuote`'s (BQ-3).
//! - `quote_in` / `quote_with_coupon_in` are `price_in(…, QuoteTerms::Indicative)`.
//!
//! The `BINDING` rows need BQ-0's migration. Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod quote_common;

use chrono::{DateTime, Duration, Utc};
use pos_error::AppResult;
use quote_common::*;
use rust_decimal::Decimal;
use std::str::FromStr;
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_exchange::domain::quote::quote_type::QuoteType;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService, QuoteTerms,
};
use xchange::application::workflows::save_step::SaveActor;

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-7".to_owned(),
        username: "user-employee-7".to_owned(),
        employee_id: Some("employee-7".to_owned()),
        standing: StandingKind::Staff(AccessRole::Teller),
    }
}

fn request(side: Side, quantity: &str) -> QuoteRequest {
    QuoteRequest {
        branch_id: BranchId::new("MAIN"),
        customer_id: None,
        channel: QuoteChannel::Desk,
        lines: vec![QuoteRequestLine {
            currency: CurrencyCode::parse("USD").expect("USD"),
            shop_side: side,
            quantity: ForeignAmount::from_decimal(Decimal::from_str(quantity).expect("a decimal")),
            fee_thb: None,
            denomination_group: xc_rate::domain::currency::denomination_groups::GroupCode::all(),
        }],
        payment_method: None,
    }
}

/// `price_in` in a transaction of its own, committed whatever the answer.
async fn price(
    db: &TestDb,
    req: QuoteRequest,
    now: DateTime<Utc>,
    terms: QuoteTerms,
) -> AppResult<Quote> {
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let got = service
        .price_in(&mut tx, req, &teller(), now, None, terms)
        .await;
    tx.commit().await.expect("commit");
    got
}

async fn stored_type(db: &TestDb, q: &Quote) -> String {
    db.text(&format!("SELECT type FROM quotes WHERE id = '{}'", q.id))
        .await
        .expect("a stored type")
}

#[tokio::test]
async fn q11_indicative_terms_are_todays_quote() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    let now = whole_second_now();
    let q = price(&db, request(Side::Sell, "100"), now, QuoteTerms::Indicative)
        .await
        .unwrap_or_else(|e| panic!("price_in: {e}"));
    assert_eq!(q.quote_type, QuoteType::Indicative);
    assert_eq!(q.expires_at, now + Duration::seconds(300));
    assert_eq!(stored_type(&db, &q).await, "INDICATIVE");
    db.finish().await;
}

#[tokio::test]
async fn bq2_binding_terms_store_binding_with_their_own_lifetime() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    db.exec(&format!(
        "UPDATE exchange_settings SET quote_ttl_seconds = 120 WHERE organization_id = '{ORG_1}'"
    ))
    .await;
    let now = whole_second_now();
    let q = price(
        &db,
        request(Side::Sell, "100"),
        now,
        QuoteTerms::Binding {
            ttl: Duration::seconds(900),
        },
    )
    .await
    .unwrap_or_else(|e| panic!("price_in: {e}"));
    assert_eq!(q.quote_type, QuoteType::Binding);
    assert_eq!(
        q.expires_at,
        now + Duration::seconds(900),
        "the lock's lifetime, not quote_ttl_seconds"
    );
    assert_eq!(stored_type(&db, &q).await, "BINDING");
    let stored_expiry = db
        .text(&format!(
            "SELECT (expires_at = '{}'::timestamptz)::text FROM quotes WHERE id = '{}'",
            (now + Duration::seconds(900)).to_rfc3339(),
            q.id
        ))
        .await;
    assert_eq!(stored_expiry.as_deref(), Some("true"), "stored expires_at");
    db.finish().await;
}

#[tokio::test]
async fn bq2_binding_and_indicative_price_the_same_lines_alike() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    let now = whole_second_now();
    for side in [Side::Sell, Side::Buy] {
        let i = price(&db, request(side, "250"), now, QuoteTerms::Indicative)
            .await
            .unwrap_or_else(|e| panic!("indicative: {e}"));
        let b = price(
            &db,
            request(side, "250"),
            now,
            QuoteTerms::Binding {
                ttl: Duration::seconds(600),
            },
        )
        .await
        .unwrap_or_else(|e| panic!("binding: {e}"));
        assert_eq!(b.publication_id, i.publication_id, "{side:?} publication");
        assert_eq!(b.lines.len(), 1);
        let (il, bl) = (&i.lines[0], &b.lines[0]);
        assert_eq!(bl.base_rate, il.base_rate, "{side:?} base rate");
        assert_eq!(bl.final_rate, il.final_rate, "{side:?} final rate");
        assert_eq!(bl.thb_amount, il.thb_amount, "{side:?} thb");
        assert_eq!(bl.settle_thb, il.settle_thb, "{side:?} settle");
    }
    db.finish().await;
}

#[tokio::test]
async fn bq2_quote_in_and_quote_with_coupon_in_still_issue_indicative() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    let now = whole_second_now();
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let a = service
        .quote_in(&mut tx, request(Side::Sell, "10"), &teller(), now)
        .await
        .unwrap_or_else(|e| panic!("quote_in: {e}"));
    let b = service
        .quote_with_coupon_in(&mut tx, request(Side::Buy, "10"), &teller(), now, None)
        .await
        .unwrap_or_else(|e| panic!("quote_with_coupon_in: {e}"));
    tx.commit().await.expect("commit");
    for q in [&a, &b] {
        assert_eq!(q.quote_type, QuoteType::Indicative);
        assert_eq!(q.expires_at, now + Duration::seconds(300));
        assert_eq!(stored_type(&db, q).await, "INDICATIVE");
    }
    db.finish().await;
}

#[tokio::test]
async fn bq2_a_stored_binding_quote_reads_back_as_binding() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    let now = whole_second_now();
    let q = price(
        &db,
        request(Side::Sell, "5"),
        now,
        QuoteTerms::Binding {
            ttl: Duration::seconds(60),
        },
    )
    .await
    .unwrap_or_else(|e| panic!("price_in: {e}"));
    let mut tx = db.pool.begin().await.expect("begin");
    let back = PgQuoteRepository::new(db.pool.clone())
        .find_in(&mut tx, q.id)
        .await
        .unwrap_or_else(|e| panic!("find_in: {e}"))
        .expect("the quote");
    tx.rollback().await.expect("rollback");
    assert_eq!(back.quote_type, QuoteType::Binding);
    assert_eq!(back.expires_at, now + Duration::seconds(60));
    db.finish().await;
}
