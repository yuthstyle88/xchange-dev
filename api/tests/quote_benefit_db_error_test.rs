//! A store failure while a quote reserves its benefit is a server error, not a refusal.
//!
//! Written before the fix by the ticket author (claude/d7aa64), on the xchange lane controller's ruling: a
//! Store/Database error passes through as `AppError::Database` (or `Internal`) and is never flattened into
//! `AppError::Domain(e.to_string())`. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The site: `QuoteService::quote_with_coupon_in` (`application/workflows/exchange/quote.rs`), the
//! `budget_ledger.reserve_in` arm `Err(e) => Err(AppError::Domain(e.to_string()))`. `e` is a `BenefitError`;
//! its `Store(AppError)` variant is `#[error(transparent)]` (`xc-marketing/.../benefit_error.rs`), so a sqlx
//! failure inside `reserve_in` reaches `commands/quotes.rs` as `AppError::Domain("database error: …")` and is
//! answered **400 `validationFailed` carrying the Postgres text**.
//!
//! How the failure is injected: `QuoteService` holds a concrete `Arc<BudgetLedger>` (no trait to fake), so
//! the failure is a real database one, confined to the test's own schema: once the campaign is seeded,
//! `benefit_reservations` gets `CHECK (false) NOT VALID`. Existing rows are not checked; the only statement
//! the quote path runs against that table that writes a row is `reserve_in`'s `INSERT` (step 6), which then
//! fails with SQLSTATE 23514. Nothing before `reserve_in` writes that table, so the failure is at exactly
//! that call; the control row (`…_without_the_fault_…`) proves the same quote otherwise reserves.
//!
//! Rows:
//! 1. `a_store_failure_reserving_the_benefit_answers_500_server_error` — status 500, code `serverError` (the
//!    `Err(e)` arm of `commands/quotes.rs`), and nothing stored. **Red on main** (400 `validationFailed`).
//! 2. `a_store_failure_reserving_the_benefit_sends_no_database_text` — the body carries none of the
//!    database's words. **Depends on #2029** (PR 2055, the generic 5xx body in `v3/error.rs`): after the fix
//!    alone the 500's message is still `e.to_string()` = `database error: …`, so this row stays red until
//!    #2029 is on main as well.
//! 3. `without_the_fault_the_same_quote_reserves_its_benefit` — control: 200, one benefit, one reservation.
//!    Green on main.
//! 4. `a_missing_budget_row_keeps_todays_refusal` — keep-pin for a `BenefitError` that is not a store
//!    failure (`NotFound`, the campaign's `campaign_budgets` row absent): today's 400 `validationFailed`
//!    stays. Green on main; red if the fix sends every `BenefitError` to 500.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.
//! Each test works in a schema of its own.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUTE: &str = "/api/quotes";
const BRANCH: &str = "MAIN";

/// Words of a Postgres/sqlx failure that must never reach the client.
const DATABASE_WORDS: [&str; 5] = [
    "database error",
    "relation",
    "constraint",
    "sqlx",
    "numeric",
];

/// A FRONT teller of MAIN, signed in for every request.
struct FrontOfMain;

impl SessionReader for FrontOfMain {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: Actor::staff("employee-FRONT", Role::Front, BranchId::new(BRANCH)),
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

/// `POST /api/quotes` for USD 100 BUY at the desk, with a fresh `Idempotency-Key`.
async fn quote_usd_100(db: &TestDb) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(FrontOfMain);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let body = json!({
        "channel": "DESK",
        "lines": [{ "currency": "USD", "shopSide": "BUY", "quantity": "100" }]
    });
    let req = atest::TestRequest::post()
        .uri(ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A schema with USD published at MAIN and one APPROVED campaign whose rule (USD, no customer condition)
/// applies to an anonymous USD quote at MAIN, with its `campaign_budgets` row. Returns the version id.
async fn shop_with_a_usd_campaign() -> (TestDb, Uuid) {
    let db = TestDb::start().await;
    db.publish(BRANCH, &[USD]).await;

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'tester-quote-db-error', 'ACTIVE', 'system')",
    )
    .bind(user_id)
    .bind(ORG_1)
    .execute(&db.pool)
    .await
    .expect("seed user");

    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, $4, now())",
    )
    .bind(campaign_id)
    .bind(ORG_1)
    .bind(unique("campaign"))
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert campaign");
    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
         budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, 'APPROVED', now() - interval '1 day', now() + interval '30 days', \
         5000.00, 'default', 1, 'hash_quote_db_error', now(), $4, now())",
    )
    .bind(version_id)
    .bind(ORG_1)
    .bind(campaign_id)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert version");
    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version_id)
        .bind(campaign_id)
        .execute(&db.pool)
        .await
        .expect("set current_version_id");
    sqlx::query("INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)")
        .bind(version_id)
        .bind(BRANCH)
        .execute(&db.pool)
        .await
        .expect("insert branch link");
    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
         calculation, caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, false, 1)",
    )
    .bind(Uuid::new_v4())
    .bind(version_id)
    .bind(json!({ "currencies": ["USD"] }))
    .bind(json!({ "type": "FIXED_RATE", "delta": "0.50" }))
    .execute(&db.pool)
    .await
    .expect("insert rule");
    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0)",
    )
    .bind(version_id)
    .execute(&db.pool)
    .await
    .expect("insert budget row");
    (db, version_id)
}

/// Every insert into `benefit_reservations` now fails in the database (SQLSTATE 23514).
async fn break_reservations(db: &TestDb) {
    db.exec(
        "ALTER TABLE benefit_reservations \
         ADD CONSTRAINT quote_db_error_fault CHECK (false) NOT VALID",
    )
    .await;
}

#[actix_web::test]
async fn a_store_failure_reserving_the_benefit_answers_500_server_error() {
    let (db, _) = shop_with_a_usd_campaign().await;
    break_reservations(&db).await;

    let a = quote_usd_100(&db).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::INTERNAL_SERVER_ERROR, "serverError"),
        "a database failure inside reserve_in is a server error, never a 400 refusal: body {}",
        a.text
    );
    assert_eq!(
        db.quote_rows().await,
        (0, 0),
        "the failed quote stores nothing"
    );
    db.finish().await;
}

/// Depends on #2029 (PR 2055): the generic 5xx body. With the reserve fix alone the 500 still says
/// `database error: …`.
#[actix_web::test]
async fn a_store_failure_reserving_the_benefit_sends_no_database_text() {
    let (db, _) = shop_with_a_usd_campaign().await;
    break_reservations(&db).await;

    let a = quote_usd_100(&db).await;
    let lower = a.text.to_lowercase();
    let leaked: Vec<&str> = DATABASE_WORDS
        .iter()
        .copied()
        .filter(|w| lower.contains(w))
        .collect();
    assert!(
        leaked.is_empty(),
        "the answer to a database failure carries none of {DATABASE_WORDS:?}; found {leaked:?} in {}",
        a.text
    );
    db.finish().await;
}

#[actix_web::test]
async fn without_the_fault_the_same_quote_reserves_its_benefit() {
    let (db, version_id) = shop_with_a_usd_campaign().await;

    let a = quote_usd_100(&db).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    let benefits = a.body["benefits"]
        .as_array()
        .unwrap_or_else(|| panic!("benefits is a list: {}", a.text));
    assert_eq!(benefits.len(), 1, "the USD rule applies: {}", a.text);
    let reserved = db
        .count(&format!(
            "SELECT count(*) FROM benefit_reservations \
             WHERE campaign_version_id = '{version_id}'::uuid AND status = 'RESERVED'"
        ))
        .await;
    assert_eq!(reserved, 1, "reserve_in ran and held the benefit");
    db.finish().await;
}

#[actix_web::test]
async fn a_missing_budget_row_keeps_todays_refusal() {
    let (db, version_id) = shop_with_a_usd_campaign().await;
    db.exec(&format!(
        "DELETE FROM campaign_budgets WHERE campaign_version_id = '{version_id}'::uuid"
    ))
    .await;

    let a = quote_usd_100(&db).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::BAD_REQUEST, "validationFailed"),
        "BenefitError::NotFound is not a store failure and keeps today's answer: body {}",
        a.text
    );
    db.finish().await;
}
