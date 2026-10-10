//! AL-2 (#1002, rate alerts RA-2 of #561) — `RateAlertService::{pause, resume, delete}` (design §2 R5, R10, R15;
//! §4.3; §9.1 row "RA-2"; ticket §8 `rateAlertPause` / `rateAlertResume` / `rateAlertDelete`, §9 edge cases).
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes (ticket §7, §8): `pause` / `resume` / `delete(&self, tx, customer_id: &str,
//! id: RateAlertId, expected_version: i32, now: DateTime<Utc>) -> Result<RateAlert, RateAlertError>`. Each success
//! raises `version` by one (`expectedVersion` is optimistic, §4.1) and answers the alert as stored.

mod common;

use common::*;

/// Run one lifecycle command in its own transaction; commit on success, roll back on a refusal.
async fn act(
    db: &TestDb,
    svc: &RateAlertService,
    what: &str,
    customer: &str,
    id: RateAlertId,
    expected_version: i32,
) -> Result<RateAlert, RateAlertError> {
    let mut tx = db.begin().await;
    let result = match what {
        "pause" => {
            svc.pause(&mut tx, customer, id, expected_version, t0())
                .await
        }
        "resume" => {
            svc.resume(&mut tx, customer, id, expected_version, t0())
                .await
        }
        "delete" => {
            svc.delete(&mut tx, customer, id, expected_version, t0())
                .await
        }
        other => panic!("unknown command {other}"),
    };
    settle(tx, result).await
}

/// A CX-1 alert created through the service: USD, CUSTOMER_BUY (shop SELL, 34.00 in v1), ABOVE `target`.
async fn created(db: &TestDb, svc: &RateAlertService, target: &str) -> RateAlert {
    db.create(
        svc,
        CX1,
        &input(
            "USD",
            CustomerSide::CustomerBuy,
            AlertCondition::Above,
            target,
        ),
        &key(),
    )
    .await
    .expect("the alert is created")
}

// ── R10 ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn pause_disables_the_alert_and_raises_its_version() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "35").await;

    let paused = act(&db, &svc, "pause", CX1, alert.id, 1)
        .await
        .expect("paused");
    assert!(!paused.enabled);
    assert_eq!(paused.version, 2);
    let row = db.row(alert.id).await;
    assert_eq!((row.enabled, row.version, row.deleted_at), (false, 2, None));
    db.finish().await;
}

#[tokio::test]
async fn resume_re_observes_the_current_publication_without_firing() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    // v1 sell 34.00 is under 35: NOT_SATISFIED at creation.
    let alert = created(&db, &svc, "35").await;
    assert_eq!(db.row(alert.id).await.relation, "NOT_SATISFIED");
    act(&db, &svc, "pause", CX1, alert.id, 1)
        .await
        .expect("paused");

    // While paused the rate crosses the target.
    db.publish(MAIN, 2, vec![line("USD", "1", "35.00", "35.50")])
        .await;

    let resumed = act(&db, &svc, "resume", CX1, alert.id, 2)
        .await
        .expect("resumed");
    assert!(resumed.enabled);
    assert_eq!(resumed.version, 3);
    let row = db.row(alert.id).await;
    assert!(row.enabled);
    assert_eq!(
        row.relation, "SATISFIED",
        "resume resets to UNKNOWN and observes the current publication (R10)"
    );
    assert_eq!(
        row.episode_no, 0,
        "the first observation after resume is not a crossing (R5)"
    );
    assert_eq!(db.events().await, 0, "resume never fires");
    db.finish().await;
}

#[tokio::test]
async fn resume_on_a_stale_side_leaves_the_relation_unknown() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "34").await;
    assert_eq!(db.row(alert.id).await.relation, "SATISFIED");
    act(&db, &svc, "pause", CX1, alert.id, 1)
        .await
        .expect("paused");
    db.publish(
        MAIN,
        2,
        vec![Line {
            snapshot: stale_auto(),
            ..line("USD", "1", "33.50", "34.00")
        }],
    )
    .await;

    act(&db, &svc, "resume", CX1, alert.id, 2)
        .await
        .expect("resumed");
    assert_eq!(
        db.row(alert.id).await.relation,
        "UNKNOWN",
        "the relation is reset, and a skipped side does not set it (R8, R10)"
    );
    db.finish().await;
}

#[tokio::test]
async fn delete_marks_the_alert_deleted_and_keeps_its_events() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "35").await;
    db.exec(&format!(
        "INSERT INTO rate_alert_events (id, organization_id, alert_id, published_rate_line_id, observed_rate, \
         trigger_at, episode_no) \
         SELECT gen_random_uuid(), '{ORG}', '{}', l.id, 35.10000000, now(), 1 \
         FROM published_rate_lines l JOIN branches b ON b.current_publication_id = l.publication_id \
         WHERE b.id = 'MAIN' AND l.currency_code = 'USD'",
        alert.id
    ))
    .await;

    let deleted = act(&db, &svc, "delete", CX1, alert.id, 1)
        .await
        .expect("deleted");
    assert_eq!(deleted.deleted_at, Some(t0()));
    assert_eq!(deleted.version, 2);
    let row = db.row(alert.id).await;
    assert_eq!(
        row.deleted_at,
        Some(t0()),
        "delete sets deleted_at; the row stays"
    );
    assert_eq!(db.events().await, 1, "events are history and stay (R10)");
    db.finish().await;
}

// R15 (only the owner) moved to `rate_alert_rulings_test.rs`: the controller ruled on #1002 that another
// customer's id is `alertNotFound`, not `outOfScope` (never reveal that someone else's alert exists).

// ── optimistic version ──────────────────────────────────────────────────────

#[tokio::test]
async fn a_stale_expected_version_is_stale_version_for_every_command() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "35").await;
    act(&db, &svc, "pause", CX1, alert.id, 1)
        .await
        .expect("paused, now version 2");
    let before = db.row(alert.id).await;

    for what in ["pause", "resume", "delete"] {
        let got = act(&db, &svc, what, CX1, alert.id, 1).await;
        assert_eq!(
            refusal(got).0,
            "staleVersion",
            "{what} with version 1 after the pause"
        );
    }
    for what in ["pause", "resume", "delete"] {
        let got = act(&db, &svc, what, CX1, alert.id, 3).await;
        assert_eq!(
            refusal(got).0,
            "staleVersion",
            "{what} with a version from the future"
        );
    }
    assert_eq!(db.row(alert.id).await, before, "a refusal changes nothing");
    db.finish().await;
}
