//! RA-1 (#979, design `docs/design/blueprint/broker-seam.md` §4.1, §4.2, §5.3, §11) — the
//! attribution capture: `ReferralCode`, `referral_attributions` and `ReferralCaptureService`.
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! BK-1 (#1377, `docs/design/blueprint/booking-slots.md` §8 M1 item 1, §9.1) re-seeded the booking captures on a
//! Blueprint booking (the new `bookings`, whose `customer_id` is NOT NULL) and replaced the anonymous-booking
//! `NoCustomer` case: from M1 on, a v2 booking id (v2's booking table) is `NoSuchTarget`. Written before BK-1's
//! implementation by the ticket author (claude/89bd61d9); the same rule applies.
//!
//! Uses FD-1's harness (`tests/common/mod.rs`): a schema per test with `api/migrations/` applied,
//! so the table comes from this ticket's migration. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;

use std::sync::Arc;

use common::{MAIN, ORG, TestDb, at, unique};
use serde_json::{Value, json};
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_referral::application::capture_service::{
    CaptureError, CaptureInput, ReferralCaptureService,
};
use xc_referral::domain::capture::{CaptureId, CaptureSource, CaptureTarget};
use xc_referral::domain::events::BROKER_ATTRIBUTION;
use xc_referral::domain::referral_code::ReferralCode;
use xc_referral::infrastructure::capture_repository::PgCaptureRepository;

// ---------------------------------------------------------------- ReferralCode

#[test]
fn a_code_is_trimmed_and_upper_cased_into_brokers_format() {
    assert_eq!(
        ReferralCode::parse("ab12cd").expect("valid").as_str(),
        "AB12CD"
    );
    assert_eq!(
        ReferralCode::parse(" AB12CD ").expect("valid").as_str(),
        "AB12CD"
    );
    assert_eq!(
        ReferralCode::parse("ABCDEFGHJK23")
            .expect("12 characters")
            .as_str(),
        "ABCDEFGHJK23"
    );
}

#[test]
fn a_code_outside_brokers_format_is_refused() {
    for bad in [
        "",
        "   ",
        "AB12C",
        "AB12CDEFGH345",
        "AB-12C",
        "AB12C!",
        "AB 12CD",
        "ÄB12CD",
    ] {
        assert!(
            matches!(ReferralCode::parse(bad), Err(CaptureError::BadCode)),
            "{bad:?} must be BadCode"
        );
    }
}

#[test]
fn the_event_kind_is_the_designs() {
    assert_eq!(BROKER_ATTRIBUTION.event_type, "broker.attribution");
    assert_eq!(BROKER_ATTRIBUTION.destination, "broker");
    assert_eq!(BROKER_ATTRIBUTION.aggregate_type, "referral_attribution");
}

// ---------------------------------------------------------------- helpers

fn service(db: &TestDb) -> ReferralCaptureService {
    ReferralCaptureService::new(
        Arc::new(PgCaptureRepository::new()),
        Arc::new(PgAuditLog::new(db.pool.clone())),
    )
}

fn code(text: &str) -> ReferralCode {
    ReferralCode::parse(text).expect("a valid code")
}

fn on_transaction(id: Uuid, text: &str, sub: Option<&str>) -> CaptureInput {
    CaptureInput {
        code: code(text),
        on: CaptureTarget::Transaction(id),
        source: CaptureSource::Desk,
        customer_sub: sub.map(str::to_owned),
        by: "teller-1".to_owned(),
        at: at(0),
    }
}

fn on_booking(id: &str, text: &str, sub: Option<&str>) -> CaptureInput {
    CaptureInput {
        code: code(text),
        on: CaptureTarget::Booking(id.to_owned()),
        source: CaptureSource::Booking,
        customer_sub: sub.map(str::to_owned),
        by: "customer:sub-x".to_owned(),
        at: at(5),
    }
}

/// A customer and one of its exchanges; answers the exchange's id.
async fn exchange_of(db: &TestDb, customer: &str) -> Uuid {
    db.seed_customer(customer).await;
    db.seed_transaction(customer).await.0
}

/// A CONFIRMED DESK booking of MAIN for `customer`, in a slot of its own (the Blueprint `bookings`, BK-1 M1);
/// answers its id.
async fn booking(db: &TestDb, customer: &str) -> String {
    let slot = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, \
             capacity, reserved, status) \
         VALUES ($1, $2::uuid, $3, DATE '2026-10-20', \
             TIMESTAMPTZ '2026-10-20 03:00:00+00' + make_interval(mins => $4), \
             TIMESTAMPTZ '2026-10-20 03:30:00+00' + make_interval(mins => $4), 4, 1, 'OPEN')",
    )
    .bind(slot)
    .bind(ORG)
    .bind(MAIN)
    .bind(i32::try_from(slot.as_u128() % 100_000).expect("fits"))
    .execute(&db.pool)
    .await
    .expect("insert into booking_slots");
    let id = unique("BK");
    sqlx::query(
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, \
             shop_side, requested_currency, requested_quantity, source_channel, status, qr_token_hash, \
             created_by) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, 'BUY', 'USD', 100, 'DESK', 'CONFIRMED', \
             decode(md5($1), 'hex'), 'teller-1')",
    )
    .bind(&id)
    .bind(ORG)
    .bind(MAIN)
    .bind(unique("B"))
    .bind(customer)
    .bind(slot)
    .execute(&db.pool)
    .await
    .expect("insert into bookings");
    id
}

async fn capture(db: &TestDb, input: CaptureInput) -> Result<CaptureId, CaptureError> {
    let mut tx = db.begin().await;
    let out = service(db).capture_in(&mut tx, input).await;
    if out.is_ok() {
        tx.commit().await.expect("commit");
    }
    out
}

/// One `referral_attributions` row, read back as JSON.
async fn row(db: &TestDb, id: &CaptureId) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(r) FROM referral_attributions r WHERE r.id = $1::uuid")
        .bind(id.to_string())
        .fetch_one(&db.pool)
        .await
        .expect("the capture row")
}

async fn count(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

async fn outbox_rows(db: &TestDb) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('org', organization_id::text, 'event_type', event_type, \
             'destination', destination, 'aggregate_type', aggregate_type, \
             'aggregate_id', aggregate_id, 'key', idempotency_key, \
             'payload', payload_redacted, 'status', status) \
         FROM outbox_events ORDER BY seq",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read outbox_events")
}

// ---------------------------------------------------------------- capture_in

#[tokio::test]
async fn a_first_capture_writes_one_active_row_read_from_its_exchange() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;

    let id = capture(&db, on_transaction(tx_id, "ab12cd", None))
        .await
        .expect("captured");
    let r = row(&db, &id).await;
    assert_eq!(r["status"], "ACTIVE");
    assert_eq!(r["code"], "AB12CD", "stored upper-cased");
    assert_eq!(r["customer_id"], "C-1", "the exchange's customer");
    assert_eq!(r["branch_id"], MAIN);
    assert_eq!(r["organization_id"], ORG);
    assert_eq!(r["transaction_id"], tx_id.to_string());
    assert_eq!(r["booking_id"], Value::Null);
    assert_eq!(r["source"], "DESK");
    assert_eq!(r["captured_by"], "teller-1");
    assert_eq!(r["customer_sub"], Value::Null);
    assert_eq!(r["supersedes_id"], Value::Null);
    assert_eq!(r["reason"], Value::Null);
    assert_eq!(r["pushed_at"], Value::Null, "no sub: nothing pushed");
    assert_eq!(r["broker_outcome"], Value::Null);
    assert_eq!(
        outbox_rows(&db).await,
        Vec::<Value>::new(),
        "no customer sub, no outbox row (D-3)"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_same_code_twice_is_a_repeat_and_another_code_is_already_captured() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let first = capture(&db, on_transaction(tx_id, "AB12CD", Some("sub-1")))
        .await
        .expect("first");

    let again = capture(&db, on_transaction(tx_id, "ab12cd", Some("sub-1")))
        .await
        .expect("the same code is a repeat, not an error");
    assert_eq!(again, first, "a repeat answers the same capture");

    let other = capture(&db, on_transaction(tx_id, "ZZ99YY", Some("sub-1"))).await;
    assert!(
        matches!(other, Err(CaptureError::AlreadyCaptured)),
        "got {other:?}"
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM referral_attributions").await,
        1
    );
    assert_eq!(
        outbox_rows(&db).await.len(),
        1,
        "the repeat pushed nothing again"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_partial_unique_indexes_refuse_a_second_active_row_too() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let on_tx = capture(&db, on_transaction(tx_id, "AB12CD", None))
        .await
        .expect("on the exchange");
    let bk = booking(&db, "C-1").await;
    let on_bk = capture(&db, on_booking(&bk, "AB12CD", None))
        .await
        .expect("on the booking");

    for (what, id) in [("transaction", on_tx), ("booking", on_bk)] {
        let err = sqlx::query(
            "INSERT INTO referral_attributions (id, organization_id, branch_id, code, customer_id, \
                 booking_id, transaction_id, source, captured_at, captured_by, status) \
             SELECT gen_random_uuid(), organization_id, branch_id, 'ZZ99YY', customer_id, \
                 booking_id, transaction_id, source, captured_at, captured_by, 'ACTIVE' \
             FROM referral_attributions WHERE id = $1::uuid",
        )
        .bind(id.to_string())
        .execute(&db.pool)
        .await
        .expect_err("a second ACTIVE row on one target");
        let code = err
            .as_database_error()
            .and_then(|e| e.code())
            .map(|c| c.into_owned());
        assert_eq!(
            code.as_deref(),
            Some("23505"),
            "{what}: a unique violation, got {err}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn a_capture_with_a_sub_writes_one_outbox_row_in_the_same_transaction() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let id = capture(&db, on_transaction(tx_id, "AB12CD", Some("sub-1")))
        .await
        .expect("captured");

    let r = row(&db, &id).await;
    assert_eq!(r["customer_sub"], "sub-1");
    assert!(r["pushed_at"].is_string(), "pushed_at is set: {r}");
    assert_eq!(
        outbox_rows(&db).await,
        vec![json!({
            "org": ORG,
            "event_type": "broker.attribution",
            "destination": "broker",
            "aggregate_type": "referral_attribution",
            "aggregate_id": id.to_string(),
            "key": format!("attr:{id}"),
            "payload": { "captureId": id.to_string() },
            "status": "PENDING",
        })]
    );
    db.finish().await;
}

#[tokio::test]
async fn a_rolled_back_capture_writes_neither_row() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let mut tx = db.begin().await;
    service(&db)
        .capture_in(&mut tx, on_transaction(tx_id, "AB12CD", Some("sub-1")))
        .await
        .expect("captured inside the caller's transaction");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        count(&db, "SELECT count(*) FROM referral_attributions").await,
        0
    );
    assert_eq!(count(&db, "SELECT count(*) FROM outbox_events").await, 0);
    db.finish().await;
}

#[tokio::test]
async fn a_booking_capture_takes_the_organisation_branch_and_customer_of_the_booking() {
    let db = TestDb::start().await;
    db.seed_customer("C-1").await;
    let bk = booking(&db, "C-1").await;
    let id = capture(&db, on_booking(&bk, "AB12CD", None))
        .await
        .expect("captured");
    let r = row(&db, &id).await;
    assert_eq!(r["customer_id"], "C-1");
    assert_eq!(r["organization_id"], ORG);
    assert_eq!(r["branch_id"], MAIN);
    assert_eq!(r["booking_id"], bk.as_str());
    assert_eq!(r["transaction_id"], Value::Null);
    assert_eq!(r["source"], "BOOKING");
    assert_eq!(r["captured_by"], "customer:sub-x");
    db.finish().await;
}

#[tokio::test]
async fn a_v2_booking_or_a_target_with_no_row_is_refused() {
    let db = TestDb::start().await;
    let v2 = unique("BK");
    let out = capture(&db, on_booking(&v2, "AB12CD", None)).await;
    assert!(
        matches!(out, Err(CaptureError::NoSuchTarget)),
        "a v2 booking is not a Blueprint booking (BK-1 M1 item 1), got {out:?}"
    );

    let out = capture(&db, on_booking("BK-nope", "AB12CD", None)).await;
    assert!(
        matches!(out, Err(CaptureError::NoSuchTarget)),
        "got {out:?}"
    );
    let out = capture(&db, on_transaction(Uuid::new_v4(), "AB12CD", None)).await;
    assert!(
        matches!(out, Err(CaptureError::NoSuchTarget)),
        "got {out:?}"
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM referral_attributions").await,
        0
    );
    db.finish().await;
}

// ---------------------------------------------------------------- supersede_in

async fn supersede(
    db: &TestDb,
    id: &CaptureId,
    text: &str,
    reason: &str,
) -> Result<CaptureId, CaptureError> {
    let mut tx = db.begin().await;
    let out = service(db)
        .supersede_in(&mut tx, id, code(text), reason, "owner-1", at(60))
        .await;
    if out.is_ok() {
        tx.commit().await.expect("commit");
    }
    out
}

#[tokio::test]
async fn supersede_without_a_reason_is_refused() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let id = capture(&db, on_transaction(tx_id, "AB12CD", None))
        .await
        .expect("captured");
    for reason in ["", "   "] {
        let out = supersede(&db, &id, "ZZ99YY", reason).await;
        assert!(
            matches!(out, Err(CaptureError::EmptyReason)),
            "{reason:?}: got {out:?}"
        );
    }
    assert_eq!(row(&db, &id).await["status"], "ACTIVE", "nothing changed");
    db.finish().await;
}

#[tokio::test]
async fn supersede_leaves_exactly_one_active_row_and_audits_the_change() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let old = capture(&db, on_transaction(tx_id, "AB12CD", None))
        .await
        .expect("captured");
    let audits_before = count(&db, "SELECT count(*) FROM audit_events").await;

    let new = supersede(&db, &old, "zz99yy", "customer named another broker")
        .await
        .expect("superseded");
    assert_ne!(new, old);

    let o = row(&db, &old).await;
    assert_eq!(o["status"], "SUPERSEDED");
    assert_eq!(o["code"], "AB12CD", "a change never updates the code (D-2)");
    let n = row(&db, &new).await;
    assert_eq!(n["status"], "ACTIVE");
    assert_eq!(n["code"], "ZZ99YY");
    assert_eq!(n["supersedes_id"], old.to_string());
    assert_eq!(n["reason"], "customer named another broker");
    assert_eq!(n["transaction_id"], tx_id.to_string(), "the same target");
    assert_eq!(n["customer_id"], "C-1");
    assert_eq!(n["captured_by"], "owner-1");
    assert_eq!(
        count(
            &db,
            &format!(
                "SELECT count(*) FROM referral_attributions \
                 WHERE transaction_id = '{tx_id}' AND status = 'ACTIVE'"
            )
        )
        .await,
        1
    );

    assert_eq!(
        count(&db, "SELECT count(*) FROM audit_events").await,
        audits_before + 1,
        "one audit event"
    );
    let audit: (String, String, String) = sqlx::query_as(
        "SELECT action, object_ref, actor_id FROM audit_events ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the audit event");
    assert_eq!(
        audit,
        (
            "supersedeReferral".to_owned(),
            format!("referral_attribution:{old}"),
            "owner-1".to_owned()
        )
    );
    db.finish().await;
}

#[tokio::test]
async fn supersede_trims_the_stored_and_audited_reason() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let old = capture(&db, on_transaction(tx_id, "AB12CD", None))
        .await
        .expect("captured");

    let new = supersede(&db, &old, "zz99yy", "  moved  ")
        .await
        .expect("superseded");
    let n = row(&db, &new).await;
    assert_eq!(n["reason"], "moved", "reason is stored trimmed");

    let audit_after: Value =
        sqlx::query_scalar("SELECT after_redacted FROM audit_events ORDER BY id DESC LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .expect("the audit event");
    assert_eq!(
        audit_after["reason"], "moved",
        "audit after stores trimmed reason"
    );

    db.finish().await;
}

#[tokio::test]
async fn superseding_a_row_that_is_no_longer_active_is_refused() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let old = capture(&db, on_transaction(tx_id, "AB12CD", None))
        .await
        .expect("captured");
    supersede(&db, &old, "ZZ99YY", "first change")
        .await
        .expect("first supersede");

    let out = supersede(&db, &old, "QQ88PP", "second change of the old row").await;
    assert!(matches!(out, Err(CaptureError::NotActive)), "got {out:?}");
    assert_eq!(
        count(&db, "SELECT count(*) FROM referral_attributions").await,
        2
    );
    db.finish().await;
}

// ---------------------------------------------------------------- on_customer_sub_known

#[tokio::test]
async fn a_known_sub_pushes_each_waiting_active_capture_once() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let bk = booking(&db, "C-1").await;
    let on_tx = capture(&db, on_transaction(tx_id, "AB12CD", None))
        .await
        .expect("on the exchange");
    let replaced = capture(&db, on_booking(&bk, "QQ88PP", None))
        .await
        .expect("on the booking");
    let on_bk = supersede(&db, &replaced, "ZZ99YY", "corrected")
        .await
        .expect("superseded: the old booking row must not be pushed");
    let other_tx = exchange_of(&db, "C-2").await;
    let other = capture(&db, on_transaction(other_tx, "AB12CD", None))
        .await
        .expect("another customer's");
    assert_eq!(outbox_rows(&db).await.len(), 0, "nothing pushed yet");

    let mut tx = db.begin().await;
    let pushed = service(&db)
        .on_customer_sub_known(&mut tx, "C-1", "sub-1")
        .await
        .expect("pushed");
    tx.commit().await.expect("commit");
    assert_eq!(pushed, 2, "the two ACTIVE captures of C-1");

    let mut aggregates: Vec<String> = outbox_rows(&db)
        .await
        .iter()
        .map(|r| r["aggregate_id"].as_str().expect("an id").to_owned())
        .collect();
    aggregates.sort();
    let mut wanted = vec![on_tx.to_string(), on_bk.to_string()];
    wanted.sort();
    assert_eq!(aggregates, wanted);
    for id in [&on_tx, &on_bk] {
        let r = row(&db, id).await;
        assert_eq!(r["customer_sub"], "sub-1");
        assert!(r["pushed_at"].is_string(), "{r}");
    }
    assert_eq!(row(&db, &replaced).await["pushed_at"], Value::Null);
    assert_eq!(
        row(&db, &other).await["customer_sub"],
        Value::Null,
        "C-2 untouched"
    );

    let mut tx = db.begin().await;
    let again = service(&db)
        .on_customer_sub_known(&mut tx, "C-1", "sub-1")
        .await
        .expect("a second call");
    tx.commit().await.expect("commit");
    assert_eq!(again, 0, "nothing is pushed twice");
    assert_eq!(outbox_rows(&db).await.len(), 2);
    db.finish().await;
}

// ---------------------------------------------------------------- link_transaction_in

#[tokio::test]
async fn linking_moves_the_bookings_capture_onto_the_exchange_and_keeps_one_row() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let bk = booking(&db, "C-1").await;
    let id = capture(&db, on_booking(&bk, "AB12CD", None))
        .await
        .expect("on the booking");

    let mut tx = db.begin().await;
    service(&db)
        .link_transaction_in(&mut tx, &bk, tx_id)
        .await
        .expect("linked");
    tx.commit().await.expect("commit");

    let r = row(&db, &id).await;
    assert_eq!(r["transaction_id"], tx_id.to_string());
    assert_eq!(r["booking_id"], bk.as_str(), "the booking stays on the row");
    assert_eq!(r["status"], "ACTIVE");
    assert_eq!(
        count(&db, "SELECT count(*) FROM referral_attributions").await,
        1
    );

    // the exchange now holds that capture: another code on it is AlreadyCaptured
    let out = capture(&db, on_transaction(tx_id, "ZZ99YY", None)).await;
    assert!(
        matches!(out, Err(CaptureError::AlreadyCaptured)),
        "got {out:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn linking_a_booking_with_no_capture_is_a_no_op() {
    let db = TestDb::start().await;
    let tx_id = exchange_of(&db, "C-1").await;
    let bk = booking(&db, "C-1").await;
    let mut tx = db.begin().await;
    service(&db)
        .link_transaction_in(&mut tx, &bk, tx_id)
        .await
        .expect("nothing to move is fine");
    tx.commit().await.expect("commit");
    assert_eq!(
        count(&db, "SELECT count(*) FROM referral_attributions").await,
        0
    );
    db.finish().await;
}
