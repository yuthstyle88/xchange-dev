//! AL-2 (#1002) rework of PR 1091 — tests added by the implementer (claude/dell-claude-3); the provided tests are
//! untouched.
//!
//! - Controller ruling 2 of issuecomment-6098735933: resume with no `rate_alert_settings` row is refused
//!   `alertSettingsMissing`, as create is, and writes nothing.
//! - Must-fix 1 of the FAIL (issuecomment-6098722904): the lifecycle rules live in `RateAlert::{pause, resume,
//!   delete}` and follow ruling 2 of issuecomment-6096995717 on their own, without the service.

mod common;

use chrono::Duration;
use common::*;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Side, UnitSize};

fn reason_of(result: Result<bool, RateAlertError>) -> &'static str {
    match result {
        Err(e) => e.reason().unwrap_or(e.code()),
        Ok(changed) => panic!("expected a refusal, got Ok({changed})"),
    }
}

/// A plain enabled alert, expiring a day after `t0()`.
fn alert() -> RateAlert {
    RateAlert {
        id: RateAlertId::new(Uuid::new_v4()),
        organization_id: org(),
        customer_id: CX1.to_owned(),
        branch: BranchId::new(MAIN),
        currency: currency("USD"),
        shop_side: Side::Sell,
        unit_size: UnitSize::ONE,
        condition: AlertCondition::Above,
        target: dec("35"),
        enabled: true,
        cooldown_seconds: 0,
        expires_at: Some(t0() + Duration::days(1)),
        last_triggered_at: None,
        relation: Relation::Satisfied,
        last_publication_version: Some(1),
        episode_no: 0,
        disabled_reason: None,
        deleted_at: None,
        version: 1,
    }
}

#[tokio::test]
async fn resuming_without_a_settings_row_is_alert_settings_missing_and_writes_nothing() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let created = db
        .create(
            &svc,
            CX1,
            &input(
                "USD",
                CustomerSide::CustomerBuy,
                AlertCondition::Above,
                "35",
            ),
            &key(),
        )
        .await
        .expect("the alert is created");
    let mut tx = db.begin().await;
    svc.pause(&mut tx, CX1, created.id, 1, t0())
        .await
        .expect("paused, version 2");
    tx.commit().await.expect("commit");
    db.exec("DELETE FROM rate_alert_settings").await;
    let before = db.row(created.id).await;

    let mut tx = db.begin().await;
    let result = svc.resume(&mut tx, CX1, created.id, 2, t0()).await;
    tx.commit().await.expect("commit after the refusal");

    let (code, reason) = refusal(result);
    assert_eq!(reason.unwrap_or(code), "alertSettingsMissing");
    assert_eq!(db.row(created.id).await, before, "nothing written");
    db.finish().await;
}

#[test]
fn resume_of_an_enabled_alert_leaves_it_exactly_as_it_was() {
    let mut a = alert();
    let before = a.clone();

    assert!(!a.resume(t0()).expect("a no-op, not a refusal"));
    assert_eq!(a, before, "the relation is kept, the version is not bumped");
}

#[test]
fn pause_of_a_paused_alert_and_delete_of_a_deleted_one_change_nothing() {
    let mut paused = alert();
    assert!(paused.pause(t0()).expect("paused"));
    let after_first = paused.clone();
    assert!(!paused.pause(t0()).expect("a no-op"));
    assert_eq!(paused, after_first);

    let mut deleted = alert();
    assert!(deleted.delete(t0()).expect("deleted"));
    let after_delete = deleted.clone();
    assert!(!deleted.delete(t0() + Duration::hours(1)).expect("a no-op"));
    assert_eq!(
        deleted, after_delete,
        "deleted_at keeps the first delete's time"
    );
}

#[test]
fn pause_and_resume_change_the_alert_and_bump_the_version_once() {
    let mut a = alert();
    assert!(a.pause(t0()).expect("paused"));
    assert!(!a.enabled);
    assert_eq!(a.version, 2);

    assert!(a.resume(t0()).expect("resumed"));
    assert!(a.enabled);
    assert_eq!(a.relation, Relation::Unknown, "the caller re-observes");
    assert_eq!(a.version, 3);
}

#[test]
fn an_expired_alert_cannot_be_paused_or_resumed() {
    let mut enabled = alert();
    enabled.expires_at = Some(t0());
    assert_eq!(reason_of(enabled.pause(t0())), "alertNotActive");

    let mut paused = alert();
    paused.enabled = false;
    paused.expires_at = Some(t0() - Duration::seconds(1));
    assert_eq!(reason_of(paused.resume(t0())), "alertNotActive");
}

#[test]
fn a_deleted_or_unit_changed_alert_cannot_be_paused_or_resumed() {
    let mut deleted = alert();
    deleted.deleted_at = Some(t0() - Duration::hours(1));
    assert_eq!(reason_of(deleted.clone().pause(t0())), "alertNotActive");
    deleted.enabled = false;
    assert_eq!(reason_of(deleted.resume(t0())), "alertNotActive");

    let mut unit_changed = alert();
    unit_changed.enabled = false;
    unit_changed.disabled_reason = Some("UNIT_CHANGED".to_owned());
    assert_eq!(
        reason_of(unit_changed.clone().resume(t0())),
        "alertNotActive"
    );
    unit_changed.enabled = true;
    assert_eq!(reason_of(unit_changed.pause(t0())), "alertNotActive");
}

#[test]
fn a_refused_command_leaves_the_alert_unchanged() {
    let mut a = alert();
    a.expires_at = Some(t0());
    let before = a.clone();

    let _ = a.pause(t0());
    let _ = a.resume(t0());

    assert_eq!(a, before);
}
