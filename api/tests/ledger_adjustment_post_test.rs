//! LA-6 up-front tests (design `docs/design/blueprint/ledger-inventory.md` §13.5 "Post", AD-2, AD-7, AD-8, §13.9
//! rows LA-T15 to LA-T23): `POST /api/v3/commands/ledger_adjustment_post`.
//!
//! 🔴 Written by the sub-controller claude/f32b85db before the implementation. Do not edit, delete or weaken;
//! add tests in a file of your own. The harness is `ledger_adjustment_common/mod.rs`: XC-B01 holds USD
//! `1000 / 32,500.00` in its safe and THB 10,000; adjustments are written as approval rows directly.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

#![allow(dead_code)]

mod common;
mod ledger_adjustment_common;

use actix_web::http::StatusCode;
use ledger_adjustment_common::*;
use rust_decimal::Decimal;
use xc_cash::application::reconcile::{Reconcile, ReconcileScope};
use xc_kernel::ids::BranchId;

/// `(account kind, currency, quantity_signed, valuation_thb)` of adjustment `id`'s entries, by kind then currency.
async fn entries(room: &Room, id: &str) -> Vec<(String, String, Decimal, Decimal)> {
    sqlx::query_as(
        "SELECT a.kind, e.currency::text, e.quantity_signed, e.valuation_thb \
           FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
           JOIN ledger_postings p ON p.id = e.posting_id \
          WHERE p.source_type = 'ADJUSTMENT' AND p.source_id = $1 AND p.source_key = 'post' \
          ORDER BY a.kind, e.currency",
    )
    .bind(id)
    .fetch_all(&room.pool)
    .await
    .expect("read the entries")
}

/// `(currency, movement_type, quantity_signed, cost_thb_signed, lot_id is null)` of adjustment `id`'s movements.
async fn movements(room: &Room, id: &str) -> Vec<(String, String, Decimal, Decimal, bool)> {
    sqlx::query_as(
        "SELECT m.currency::text, m.movement_type, m.quantity_signed, m.cost_thb_signed, m.lot_id IS NULL \
           FROM inventory_movements m JOIN ledger_postings p ON p.id = m.posting_id \
          WHERE p.source_type = 'ADJUSTMENT' AND p.source_id = $1 ORDER BY m.id",
    )
    .bind(id)
    .fetch_all(&room.pool)
    .await
    .expect("read the movements")
}

async fn negative_stock_requests(room: &Room) -> i64 {
    room.count("SELECT count(*) FROM approval_requests WHERE type = 'NEGATIVE_STOCK'")
        .await
}

fn line<'a>(a: &'a Answer, currency: &str) -> &'a serde_json::Value {
    a.body["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("lines is an array: {}", a.text))
        .iter()
        .find(|l| l["currency"] == currency)
        .unwrap_or_else(|| panic!("a {currency} line: {}", a.text))
}

// ── LA-T15 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t15_an_approved_shortfall_posts_safe_to_variance_at_the_average() {
    let room = Room::start().await;
    let id = room
        .adjustment("APPROVED", quantity_payload(B1, &[("USD", "-100")]))
        .await;
    let version = room.version(B1).await;

    let a = post(&room, FIN_A, B1, &id, &unique("k")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["id"], id.as_str(), "{}", a.text);
    let posting_id: String = sqlx::query_scalar(
        "SELECT id::text FROM ledger_postings WHERE source_type = 'ADJUSTMENT' AND source_id = $1 AND source_key = 'post'",
    )
    .bind(&id)
    .fetch_one(&room.pool)
    .await
    .expect("exactly one posting (ADJUSTMENT, id, post)");
    assert_eq!(a.body["postingId"], posting_id.as_str(), "{}", a.text);
    let usd = line(&a, "USD");
    assert_eq!(usd["quantity"], "-100", "{}", a.text);
    assert_eq!(usd["valuationThb"], "3250.00", "{}", a.text);

    assert_eq!(
        entries(&room, &id).await,
        vec![
            ("SAFE".into(), "USD".into(), d("-100"), d("-3250.00")),
            ("VARIANCE".into(), "USD".into(), d("100"), d("3250.00")),
        ]
    );
    assert_eq!(
        movements(&room, &id).await,
        vec![(
            "USD".into(),
            "ADJUST".into(),
            d("-100"),
            d("-3250.00"),
            true
        )],
        "one ADJUST at the average, never a DISPOSE"
    );
    assert_eq!(
        room.position(B1, "USD").await,
        Some((d("900"), d("29250.00")))
    );

    let audits = room.audit_rows(POST_AUDIT).await;
    assert_eq!(
        audits.len(),
        1,
        "one LEDGER_ADJUSTMENT_POST row: {audits:?}"
    );
    let (actor, object_ref, branch, _after) = &audits[0];
    assert_eq!(actor, &user_id(FIN_A));
    assert_eq!(object_ref, &format!("ledger_adjustment:{id}"));
    assert_eq!(branch.as_deref(), Some(B1));
    assert!(
        room.version(B1).await > version,
        "B01's branch version moved"
    );
    room.finish().await;
}

// ── LA-T16 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t16_an_approved_overage_posts_variance_to_safe_with_no_lot() {
    let room = Room::start().await;
    let lots = room.count("SELECT count(*) FROM inventory_lots").await;
    let id = room
        .adjustment(
            "APPROVED",
            quantity_payload(B1, &[("USD", "50"), ("THB", "250.00")]),
        )
        .await;
    let a = post(&room, FIN_B, B1, &id, &unique("k")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(line(&a, "USD")["valuationThb"], "1625.00", "{}", a.text);
    assert_eq!(line(&a, "THB")["valuationThb"], "250.00", "{}", a.text);

    assert_eq!(
        entries(&room, &id).await,
        vec![
            ("SAFE".into(), "THB".into(), d("250.00"), d("250.00")),
            ("SAFE".into(), "USD".into(), d("50"), d("1625.00")),
            ("VARIANCE".into(), "THB".into(), d("-250.00"), d("-250.00")),
            ("VARIANCE".into(), "USD".into(), d("-50"), d("-1625.00")),
        ]
    );
    assert_eq!(
        room.count("SELECT count(*) FROM inventory_lots").await,
        lots,
        "an ADJUST in opens no lot"
    );
    assert_eq!(
        room.position(B1, "USD").await,
        Some((d("1050"), d("34125.00")))
    );
    assert_eq!(room.position(B1, "THB").await, None, "THB has no position");
    room.finish().await;
}

// ── LA-T17 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t17_reconcile_stays_clean_after_both_posts() {
    let room = Room::start().await;
    let short = room
        .adjustment("APPROVED", quantity_payload(B1, &[("USD", "-100")]))
        .await;
    let over = room
        .adjustment(
            "APPROVED",
            quantity_payload(B1, &[("USD", "50"), ("THB", "250.00")]),
        )
        .await;
    for id in [&short, &over] {
        let a = post(&room, FIN_A, B1, id, &unique("k")).await;
        assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    }
    let report = Reconcile::standard()
        .run(
            &room.pool,
            &ReconcileScope {
                organization_id: ORG.to_owned(),
                branch: Some(BranchId::new(B1)),
                period: None,
            },
        )
        .await
        .expect("the standard run");
    assert_eq!(report.checks.len(), 5, "{report:?}");
    for c in &report.checks {
        assert!(c.ok, "check {} stays clean: {:?}", c.id, c.offenders);
    }
    room.finish().await;
}

// ── LA-T18 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t18_posting_twice_writes_once_and_answers_the_same_posting() {
    let room = Room::start().await;
    let id = room
        .adjustment("APPROVED", quantity_payload(B1, &[("USD", "-100")]))
        .await;
    let key = unique("k");
    let first = post(&room, FIN_A, B1, &id, &key).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let replay = post(&room, FIN_A, B1, &id, &key).await;
    assert_eq!(replay.status, StatusCode::OK, "{}", replay.text);
    let fresh = post(&room, FIN_B, B1, &id, &unique("k")).await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.text);
    assert_eq!(replay.body["postingId"], first.body["postingId"]);
    assert_eq!(
        fresh.body["postingId"], first.body["postingId"],
        "a new key finds the posting by its source"
    );
    assert_eq!(room.postings_of(&id).await, 1, "one posting");
    assert_eq!(movements(&room, &id).await.len(), 1, "one movement");
    assert_eq!(
        room.position(B1, "USD").await,
        Some((d("900"), d("29250.00")))
    );
    room.finish().await;
}

// ── LA-T19 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t19_a_pending_or_rejected_adjustment_does_not_post() {
    let room = Room::start().await;
    let pending = room
        .adjustment("PENDING", quantity_payload(B1, &[("USD", "-100")]))
        .await;
    let rejected = room
        .adjustment("REJECTED", quantity_payload(B1, &[("USD", "-100")]))
        .await;
    let before = room.written().await;
    let a = post(&room, FIN_A, B1, &pending, &unique("k")).await;
    assert_refused(
        &a,
        StatusCode::CONFLICT,
        "approvalRequired",
        "a PENDING adjustment",
    );
    let a = post(&room, FIN_A, B1, &rejected, &unique("k")).await;
    assert_refused(
        &a,
        StatusCode::CONFLICT,
        "staleVersion",
        "a REJECTED adjustment",
    );
    assert_eq!(room.written().await, before, "nothing written");
    assert_eq!(room.audits(POST_AUDIT).await, 0);
    room.finish().await;
}

// ── LA-T20 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t20_a_shortfall_beyond_the_safe_is_refused_and_opens_no_negative_stock_request() {
    let room = Room::start().await;
    let id = room
        .adjustment("APPROVED", quantity_payload(B1, &[("USD", "-1200")]))
        .await;
    let before = room.written().await;
    let a = post(&room, FIN_A, B1, &id, &unique("k")).await;
    assert_refused(
        &a,
        StatusCode::CONFLICT,
        "stockInsufficient",
        "USD −1,200 against a safe of 1,000",
    );
    assert!(
        a.text.contains("200"),
        "the refusal carries the shortfall (200): {}",
        a.text
    );
    assert_eq!(room.written().await, before, "nothing written");
    assert_eq!(
        negative_stock_requests(&room).await,
        0,
        "no NEGATIVE_STOCK request (AD-7)"
    );
    assert_eq!(
        room.position(B1, "USD").await,
        Some((d("1000"), d("32500.00")))
    );
    room.finish().await;
}

// ── LA-T21 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t21_another_branch_cannot_post_this_branchs_adjustment() {
    let room = Room::start().await;
    let id = room
        .adjustment("APPROVED", quantity_payload(B1, &[("USD", "-100")]))
        .await;
    let before = room.written().await;
    let a = post(&room, FIN_ORG, B2, &id, &unique("k")).await;
    assert_refused(
        &a,
        StatusCode::NOT_FOUND,
        "notFound",
        "XC-B01's adjustment posted as XC-B02",
    );
    let unknown = post(
        &room,
        FIN_A,
        B1,
        "ADJ-00000000-0000-4000-8000-000000000000",
        &unique("k"),
    )
    .await;
    assert_refused(&unknown, StatusCode::NOT_FOUND, "notFound", "an unknown id");
    assert_eq!(room.written().await, before, "nothing written");
    room.finish().await;
}

// ── LA-T22 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t22_reservations_do_not_hold_back_a_down_adjustment() {
    let room = Room::start().await;
    room.reserve_usd("950").await;
    let id = room
        .adjustment("APPROVED", quantity_payload(B1, &[("USD", "-100")]))
        .await;
    let a = post(&room, FIN_A, B1, &id, &unique("k")).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "posted despite 950 reserved (AD-8): {}",
        a.text
    );
    assert_eq!(
        room.position(B1, "USD").await,
        Some((d("900"), d("29250.00")))
    );
    room.finish().await;
}

// ── LA-T23 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t23_an_overage_with_no_cost_history_and_no_buy_rate_is_refused() {
    let room = Room::start().await;
    let id = room
        .adjustment("APPROVED", quantity_payload(B1, &[("EUR", "10")]))
        .await;
    let before = room.written().await;
    let a = post(&room, FIN_A, B1, &id, &unique("k")).await;
    assert_refused(
        &a,
        StatusCode::BAD_REQUEST,
        "validationFailed",
        "EUR +10 with no EUR history and no published BUY rate",
    );
    assert_eq!(room.written().await, before, "nothing written");
    assert_eq!(room.position(B1, "EUR").await, None);
    room.finish().await;
}
