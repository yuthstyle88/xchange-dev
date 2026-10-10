//! AL-2 (#1002, rate alerts RA-2 of #561) — the lane controller's rulings 2 and 3 on #1002 (issue comment after
//! issuecomment-6096989377), for the cases the design does not state.
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.
//!
//! Ruling 2 (lifecycle):
//! - pause on a paused alert, resume on an enabled one, delete on a deleted one: idempotent no-op — `Ok`, the alert
//!   answered as stored, nothing written (the version does not move);
//! - pause/resume on a deleted, expired or `UNIT_CHANGED` alert: refused `alertNotActive`, nothing written;
//! - any command on an unknown id or another customer's id: `alertNotFound` (never reveal that someone else's alert
//!   exists). This replaces the `outOfScope` test that was in `rate_alert_lifecycle_test.rs`;
//! - create with `expiresAt` at or before `now`: refused `expiresAtNotInFuture`, nothing written.
//!
//! Ruling 3: create and resume set `last_publication_version` to the branch's current publication version, so an
//! alert is evaluated (and can fire) only for publications after it was created or resumed: `lock_due_in` at that
//! version or older never returns it, and at the next version it does.
//!
//! The rulings name the refusals (`alertNotActive`, `alertNotFound`, `expiresAtNotInFuture`) without saying
//! whether each is the error's code or the reason of a `validationFailed`. These tests accept either:
//! `reason()` when it is `Some`, else `code()`, must be the named word.

mod common;

use chrono::Duration;
use common::*;
use uuid::Uuid;
use xc_kernel::ids::BranchId;

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

/// The ruling's word for a refusal: the reason when there is one, else the code.
fn named<T: std::fmt::Debug>(result: Result<T, RateAlertError>) -> &'static str {
    let (code, reason) = refusal(result);
    reason.unwrap_or(code)
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

async fn due(db: &TestDb, version: i32) -> Vec<RateAlertId> {
    let repo = db.repository();
    let mut tx = db.begin().await;
    let alerts = repo
        .lock_due_in(&mut tx, &BranchId::new(MAIN), version)
        .await
        .expect("lock_due_in reads");
    tx.rollback().await.expect("roll back");
    alerts.into_iter().map(|a| a.id).collect()
}

// ── ruling 2: idempotent no-ops ─────────────────────────────────────────────

#[tokio::test]
async fn pausing_a_paused_alert_is_a_no_op() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "35").await;
    act(&db, &svc, "pause", CX1, alert.id, 1)
        .await
        .expect("paused, version 2");
    let before = db.row(alert.id).await;

    let again = act(&db, &svc, "pause", CX1, alert.id, 2)
        .await
        .expect("a second pause is not a refusal");
    assert!(!again.enabled);
    assert_eq!(again.version, 2, "a no-op does not raise the version");
    assert_eq!(db.row(alert.id).await, before, "a no-op writes nothing");
    db.finish().await;
}

#[tokio::test]
async fn resuming_an_enabled_alert_is_a_no_op() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "34").await;
    let before = db.row(alert.id).await;

    let again = act(&db, &svc, "resume", CX1, alert.id, 1)
        .await
        .expect("resuming an enabled alert is not a refusal");
    assert!(again.enabled);
    assert_eq!(again.version, 1);
    assert_eq!(
        db.row(alert.id).await,
        before,
        "a no-op writes nothing: the relation is not reset either"
    );
    db.finish().await;
}

#[tokio::test]
async fn deleting_a_deleted_alert_is_a_no_op() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "35").await;
    act(&db, &svc, "delete", CX1, alert.id, 1)
        .await
        .expect("deleted, version 2");
    let before = db.row(alert.id).await;

    let mut tx = db.begin().await;
    let later = t0() + Duration::hours(1);
    let again = svc.delete(&mut tx, CX1, alert.id, 2, later).await;
    let again = settle(tx, again)
        .await
        .expect("a second delete is not a refusal");
    assert_eq!(again.version, 2);
    assert_eq!(
        again.deleted_at,
        Some(t0()),
        "deleted_at keeps the first delete's time"
    );
    assert_eq!(db.row(alert.id).await, before, "a no-op writes nothing");
    db.finish().await;
}

// ── ruling 2: alertNotActive ────────────────────────────────────────────────

#[tokio::test]
async fn pause_and_resume_of_a_deleted_alert_are_alert_not_active() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "35").await;
    act(&db, &svc, "delete", CX1, alert.id, 1)
        .await
        .expect("deleted, version 2");
    let before = db.row(alert.id).await;

    for what in ["pause", "resume"] {
        let got = act(&db, &svc, what, CX1, alert.id, 2).await;
        assert_eq!(named(got), "alertNotActive", "{what} of a deleted alert");
    }
    assert_eq!(db.row(alert.id).await, before, "a refusal changes nothing");
    db.finish().await;
}

#[tokio::test]
async fn pause_and_resume_of_an_expired_alert_are_alert_not_active() {
    let db = TestDb::world().await;
    let svc = db.service();
    // Expired under any clock, one enabled and one paused.
    let enabled = db
        .insert_alert(CX1, MAIN, &[("expires_at", "'2020-01-01T00:00:00Z'")])
        .await;
    let paused = db
        .insert_alert(
            CX1,
            MAIN,
            &[
                ("expires_at", "'2020-01-01T00:00:00Z'"),
                ("enabled", "false"),
            ],
        )
        .await;
    let before = (db.row(enabled).await, db.row(paused).await);

    assert_eq!(
        named(act(&db, &svc, "pause", CX1, enabled, 1).await),
        "alertNotActive"
    );
    assert_eq!(
        named(act(&db, &svc, "resume", CX1, paused, 1).await),
        "alertNotActive"
    );
    assert_eq!((db.row(enabled).await, db.row(paused).await), before);
    db.finish().await;
}

#[tokio::test]
async fn pause_and_resume_of_a_unit_changed_alert_are_alert_not_active() {
    let db = TestDb::world().await;
    let svc = db.service();
    let alert = db
        .insert_alert(
            CX1,
            MAIN,
            &[("enabled", "false"), ("disabled_reason", "'UNIT_CHANGED'")],
        )
        .await;
    let before = db.row(alert).await;

    for what in ["pause", "resume"] {
        let got = act(&db, &svc, what, CX1, alert, 1).await;
        assert_eq!(
            named(got),
            "alertNotActive",
            "{what} of a UNIT_CHANGED alert"
        );
    }
    assert_eq!(db.row(alert).await, before);
    db.finish().await;
}

// ── ruling 2: alertNotFound ─────────────────────────────────────────────────

#[tokio::test]
async fn another_customers_alert_is_alert_not_found_for_every_command() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "35").await;
    let before = db.row(alert.id).await;

    for what in ["pause", "resume", "delete"] {
        let got = act(&db, &svc, what, CX2, alert.id, 1).await;
        assert_eq!(
            named(got),
            "alertNotFound",
            "{what} by CX-2 of CX-1's alert: never reveal that it exists"
        );
    }
    assert_eq!(db.row(alert.id).await, before, "a refusal changes nothing");
    db.finish().await;
}

#[tokio::test]
async fn an_unknown_id_is_alert_not_found_for_every_command() {
    let db = TestDb::world().await;
    let svc = db.service();
    let nobody = RateAlertId::new(Uuid::new_v4());
    for what in ["pause", "resume", "delete"] {
        let got = act(&db, &svc, what, CX1, nobody, 1).await;
        assert_eq!(
            named(got),
            "alertNotFound",
            "{what} of an id that does not exist"
        );
    }
    db.finish().await;
}

// ── ruling 2: expiresAtNotInFuture ──────────────────────────────────────────

#[tokio::test]
async fn an_expires_at_at_or_before_now_is_refused() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    for at in [t0() - Duration::seconds(1), t0()] {
        let got = db
            .create(
                &svc,
                CX1,
                &CreateRateAlert {
                    expires_at: Some(at),
                    ..input(
                        "USD",
                        CustomerSide::CustomerBuy,
                        AlertCondition::Above,
                        "35",
                    )
                },
                &key(),
            )
            .await;
        assert_eq!(named(got), "expiresAtNotInFuture", "expiresAt {at}");
    }
    assert_eq!(db.alerts_of(CX1).await, 0, "a refusal writes nothing");
    db.create(
        &svc,
        CX1,
        &CreateRateAlert {
            expires_at: Some(t0() + Duration::seconds(1)),
            ..input(
                "USD",
                CustomerSide::CustomerBuy,
                AlertCondition::Above,
                "35",
            )
        },
        &key(),
    )
    .await
    .expect("one second after now is in the future");
    db.finish().await;
}

// ── ruling 3: last_publication_version ──────────────────────────────────────

#[tokio::test]
async fn create_sets_the_current_publication_version_so_only_later_publications_are_due() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    db.publish(MAIN, 2, vec![line("USD", "1", "33.50", "34.00")])
        .await;
    let svc = db.service();
    let alert = created(&db, &svc, "35").await;

    assert_eq!(alert.last_publication_version, Some(2));
    assert_eq!(db.row(alert.id).await.last_publication_version, Some(2));
    assert!(
        !due(&db, 2).await.contains(&alert.id),
        "the publication current at creation is never evaluated again, so it can never fire"
    );
    assert!(!due(&db, 1).await.contains(&alert.id), "nor an older one");
    assert!(
        due(&db, 3).await.contains(&alert.id),
        "a publication after creation is evaluated, so its crossing can fire"
    );
    db.finish().await;
}

#[tokio::test]
async fn create_on_a_stale_side_still_sets_the_current_publication_version() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    db.publish(
        MAIN,
        2,
        vec![Line {
            snapshot: stale_auto(),
            ..line("USD", "1", "33.50", "34.00")
        }],
    )
    .await;
    let svc = db.service();
    let alert = created(&db, &svc, "34").await;
    let row = db.row(alert.id).await;
    assert_eq!(
        (row.relation.as_str(), row.last_publication_version),
        ("UNKNOWN", Some(2)),
        "a skipped side sets no relation, but the alert still starts after the current publication"
    );
    db.finish().await;
}

#[tokio::test]
async fn resume_sets_the_current_publication_version_so_publications_while_paused_are_never_due() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = created(&db, &svc, "35").await;
    assert_eq!(db.row(alert.id).await.last_publication_version, Some(1));
    act(&db, &svc, "pause", CX1, alert.id, 1)
        .await
        .expect("paused");

    // Two publications while paused; the second crosses the target.
    db.publish(MAIN, 2, vec![line("USD", "1", "33.50", "34.50")])
        .await;
    db.publish(MAIN, 3, vec![line("USD", "1", "35.00", "35.50")])
        .await;

    let resumed = act(&db, &svc, "resume", CX1, alert.id, 2)
        .await
        .expect("resumed");
    assert_eq!(resumed.last_publication_version, Some(3));
    assert_eq!(db.row(alert.id).await.last_publication_version, Some(3));
    for v in [2, 3] {
        assert!(
            !due(&db, v).await.contains(&alert.id),
            "v{v}, published while paused, is never evaluated after resume"
        );
    }
    assert!(
        due(&db, 4).await.contains(&alert.id),
        "the next publication is"
    );
    assert_eq!(db.events().await, 0);
    db.finish().await;
}
