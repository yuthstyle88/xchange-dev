//! RF-1 (design `docs/design/blueprint/rates.md` §4.10, PR 843): the screens and the quote path read freshness through
//! **one** rule. The `published_rates` view and `PgPublishedRates::line_in` (which every quoting reader and the kiosk's
//! `check_stale_in` read) must agree on every line; and neither view computes staleness on its own.
//!
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or weaken these tests;
//! add your own in another file.
//!
//! The publications here are seeded by SQL with a crafted `source_snapshot` (the keys the publisher writes: `buy_mode`,
//! `sell_mode`, `fetched_at`, `max_reference_age_hours`, `source_date`), so the cases do not depend on the draft flow.
//! The clock is the wall clock: references are 30 h or 1 h old against a 24 h limit.
//!
//! `TEST_DATABASE_URL` must name a database whose name ends in `_test`; each test gets its own schema.

mod auth_http_common;
mod common;

use actix_web::http::Method;
use auth_http_common::*;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};
use xc_rate::domain::publication::published_rates::PublishedRates;
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

const MAIN: &str = "MAIN";

struct Line {
    currency: &'static str,
    buy_mode: &'static str,
    sell_mode: &'static str,
    fetched_hours_ago: Option<i64>,
    max_hours: Option<i64>,
    source_date: Option<&'static str>,
}

/// One publication on MAIN holding `lines`, made current. Bypasses the draft flow on purpose: the snapshot is the
/// input under test.
async fn publish(pool: &PgPool, lines: &[Line]) {
    let request: String = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, 'rf1-seed', 'h', 'p', '{}'::jsonb, 'test', now(), \
         'APPROVED', gen_random_uuid()::text) RETURNING id::text",
    )
    .bind(MAIN)
    .fetch_one(pool)
    .await
    .expect("an approved request");
    let publication: String = sqlx::query_scalar(
        "INSERT INTO rate_publications (id, branch_id, version, approved_request_id, published_by) \
         VALUES (gen_random_uuid(), $1, \
                 (SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $1), \
                 $2::uuid, 'test') RETURNING id::text",
    )
    .bind(MAIN)
    .bind(&request)
    .fetch_one(pool)
    .await
    .expect("a publication");
    for l in lines {
        let mut snapshot = json!({ "buy_mode": l.buy_mode, "sell_mode": l.sell_mode });
        if let Some(h) = l.fetched_hours_ago {
            snapshot["fetched_at"] = json!((Utc::now() - Duration::hours(h)).to_rfc3339());
        }
        if let Some(m) = l.max_hours {
            snapshot["max_reference_age_hours"] = json!(m);
        }
        if let Some(d) = l.source_date {
            snapshot["source_date"] = json!(d);
        }
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, \
             source_snapshot) VALUES (gen_random_uuid(), $1::uuid, $2, 1, 32.10000000, 32.50000000, $3)",
        )
        .bind(&publication)
        .bind(l.currency)
        .bind(&snapshot)
        .execute(pool)
        .await
        .expect("a line");
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
        .bind(&publication)
        .bind(MAIN)
        .execute(pool)
        .await
        .expect("make it current");
}

/// The screen's answer: `GET /api/v3/views/published_rates` as the owner, `data`.
async fn screen(app: &TestApp, owner: &str) -> Value {
    let answer = send(
        app,
        Call {
            method: Some(Method::GET),
            path: "/api/v3/views/published_rates".to_owned(),
            proto: Some("https".to_owned()),
            ..Call::default()
        }
        .owner(owner),
    )
    .await;
    assert_eq!(answer.status, 200, "published_rates: {}", answer.text);
    answer.json()["data"].clone()
}

/// The quote path's answer for one side.
async fn quote_path(pool: &PgPool, currency: &str, side: Side) -> bool {
    let mut tx = pool.begin().await.expect("a transaction");
    let got = PgPublishedRates::new(pool.clone())
        .line_in(
            &mut tx,
            &BranchId::new(MAIN),
            CurrencyCode::parse(currency).unwrap(),
            side,
        )
        .await
        .expect("line_in")
        .unwrap_or_else(|| panic!("{currency} {side:?} is published"));
    tx.rollback().await.expect("roll back");
    got.stale
}

async fn setup() -> (Desk, TestApp, String) {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;
    (desk, app, owner)
}

/// §4.10: the case the two old rules disagreed on. Both sides MANUAL, but the line carries a reference past its age.
/// The quote path calls it stale; the screen must too.
#[actix_web::test]
async fn a_manual_line_carrying_an_old_reference_is_stale_on_the_screen_and_on_the_quote_path() {
    let (desk, app, owner) = setup().await;
    publish(
        &desk.pool,
        &[Line {
            currency: "USD",
            buy_mode: "MANUAL",
            sell_mode: "MANUAL",
            fetched_hours_ago: Some(30),
            max_hours: Some(24),
            source_date: Some("2026-10-08"),
        }],
    )
    .await;

    assert!(
        quote_path(&desk.pool, "USD", Side::Buy).await,
        "quote path, buy"
    );
    assert!(
        quote_path(&desk.pool, "USD", Side::Sell).await,
        "quote path, sell"
    );
    let data = screen(&app, &owner).await;
    assert_eq!(
        data["stale"],
        json!(true),
        "the screen agrees with the quote path: {data}"
    );
    drop(app);
    desk.drop_schema().await;
}

/// A fresh AUTO line and a MANUAL line with no reference are fresh on both paths.
#[actix_web::test]
async fn fresh_lines_are_fresh_on_both_paths() {
    let (desk, app, owner) = setup().await;
    publish(
        &desk.pool,
        &[
            Line {
                currency: "EUR",
                buy_mode: "MANUAL",
                sell_mode: "MANUAL",
                fetched_hours_ago: None,
                max_hours: Some(24),
                source_date: None,
            },
            Line {
                currency: "USD",
                buy_mode: "AUTO",
                sell_mode: "AUTO",
                fetched_hours_ago: Some(1),
                max_hours: Some(24),
                source_date: Some("2026-10-10"),
            },
        ],
    )
    .await;

    for (c, s) in [
        ("EUR", Side::Buy),
        ("EUR", Side::Sell),
        ("USD", Side::Buy),
        ("USD", Side::Sell),
    ] {
        assert!(!quote_path(&desk.pool, c, s).await, "quote path {c} {s:?}");
    }
    let data = screen(&app, &owner).await;
    assert_eq!(data["stale"], json!(false), "{data}");
    drop(app);
    desk.drop_schema().await;
}

/// §4.10: the publication's `sourceDate` is the **oldest** among its lines, not the first line's (EUR sorts first).
#[actix_web::test]
async fn the_screens_source_date_is_the_oldest_reference() {
    let (desk, app, owner) = setup().await;
    publish(
        &desk.pool,
        &[
            Line {
                currency: "EUR",
                buy_mode: "AUTO",
                sell_mode: "AUTO",
                fetched_hours_ago: Some(1),
                max_hours: Some(24),
                source_date: Some("2026-10-09"),
            },
            Line {
                currency: "USD",
                buy_mode: "AUTO",
                sell_mode: "AUTO",
                fetched_hours_ago: Some(2),
                max_hours: Some(24),
                source_date: Some("2026-10-06"),
            },
        ],
    )
    .await;

    assert_eq!(
        screen(&app, &owner).await["sourceDate"],
        json!("2026-10-06")
    );
    drop(app);
    desk.drop_schema().await;
}

// ── one rule, in the source ───────────────────────────────────────────────

fn source(path: &str) -> String {
    let full = format!("{}/{path}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{full}: {e}"))
}

/// Neither view nor `line_in` computes staleness itself: each calls `Freshness`, and none reads the age keys.
#[test]
fn the_views_and_the_quote_path_call_the_one_rule() {
    for path in [
        "src/presentation/http/v3/views/published_rates.rs",
        "src/presentation/http/v3/views/staff_published_rates.rs",
        "crates/xc-rate/src/infrastructure/publication/published_rates.rs",
    ] {
        let text = source(path);
        assert!(text.contains("Freshness::"), "{path} calls Freshness");
        for own in ["max_reference_age_hours", "Duration::hours", "num_seconds"] {
            assert!(
                !text.contains(own),
                "{path} computes staleness itself ({own})"
            );
        }
    }
}
