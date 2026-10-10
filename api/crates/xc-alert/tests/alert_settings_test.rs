//! AL-2 (#1002, rate alerts RA-2 of #561) — `RateAlertService::save_settings` (design §0, §2 R14, §4.1, §8 M1
//! item 1; ticket §8 `rateAlertSettingsSave`).
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.
//!
//! Signature this test assumes: `save_settings(&self, tx, organization_id: Uuid, input: &SaveAlertSettings,
//! updated_by: Uuid, now: DateTime<Utc>) -> Result<AlertSettings, RateAlertError>`. It inserts the organisation's
//! one row or replaces it. Who may call it (EXECUTIVE only) is the area row's decision (PL-0 seam), not tested here.

mod common;

use chrono::{DateTime, Duration, Utc};
use common::*;
use sqlx::Row;

fn owners() -> SaveAlertSettings {
    SaveAlertSettings {
        default_cooldown_seconds: 86400,
        max_active_per_customer: 10,
        default_lifetime_days: Some(30),
        observe_policy: ObservePolicy::AllFresh,
    }
}

async fn save(
    db: &TestDb,
    svc: &RateAlertService,
    input: &SaveAlertSettings,
) -> Result<AlertSettings, RateAlertError> {
    let mut tx = db.begin().await;
    let result = svc.save_settings(&mut tx, org(), input, exec(), t0()).await;
    settle(tx, result).await
}

/// `(cooldown, max, lifetime, policy, updated_by, updated_at)` of ORG's row.
async fn stored(db: &TestDb) -> (i32, i32, Option<i32>, String, uuid::Uuid, DateTime<Utc>) {
    let r = sqlx::query(
        "SELECT default_cooldown_seconds, max_active_per_customer, default_lifetime_days, observe_policy, \
         updated_by, updated_at FROM rate_alert_settings WHERE organization_id = $1",
    )
    .bind(org())
    .fetch_one(&db.pool)
    .await
    .expect("ORG's settings row");
    (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5))
}

#[tokio::test]
async fn the_first_save_writes_the_one_row_with_who_and_when() {
    let db = TestDb::world().await;
    let svc = db.service();
    let saved = save(&db, &svc, &owners()).await.expect("saved");
    assert_eq!(saved.organization_id, org());
    assert_eq!(saved.observe_policy, ObservePolicy::AllFresh);
    assert_eq!(
        stored(&db).await,
        (86400, 10, Some(30), "ALL_FRESH".to_owned(), exec(), t0()),
        "the owner's 2026-10-10 values, written by a person (§0, §8 M1)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_second_save_replaces_the_row() {
    let db = TestDb::world().await;
    let svc = db.service();
    save(&db, &svc, &owners()).await.expect("first");
    save(
        &db,
        &svc,
        &SaveAlertSettings {
            default_cooldown_seconds: 3600,
            max_active_per_customer: 20,
            default_lifetime_days: None,
            observe_policy: ObservePolicy::AutoFresh,
        },
    )
    .await
    .expect("second");
    assert_eq!(
        db.count("SELECT count(*) FROM rate_alert_settings").await,
        1,
        "one row per organisation"
    );
    let (cooldown, max, lifetime, policy, _, _) = stored(&db).await;
    assert_eq!(
        (cooldown, max, lifetime, policy.as_str()),
        (3600, 20, None, "AUTO_FRESH")
    );
    db.finish().await;
}

#[tokio::test]
async fn out_of_range_values_are_refused_validation_failed() {
    let db = TestDb::world().await;
    let svc = db.service();
    let bad = [
        SaveAlertSettings {
            default_cooldown_seconds: -1,
            ..owners()
        },
        SaveAlertSettings {
            max_active_per_customer: 0,
            ..owners()
        },
        SaveAlertSettings {
            default_lifetime_days: Some(0),
            ..owners()
        },
    ];
    for input in &bad {
        let got = save(&db, &svc, input).await;
        assert_eq!(refusal(got).0, "validationFailed", "{input:?}");
    }
    assert_eq!(
        db.count("SELECT count(*) FROM rate_alert_settings").await,
        0,
        "a refusal writes nothing"
    );

    save(
        &db,
        &svc,
        &SaveAlertSettings {
            default_cooldown_seconds: 0,
            ..owners()
        },
    )
    .await
    .expect("a zero cooldown is allowed (CHECK >= 0)");
    db.finish().await;
}

#[tokio::test]
async fn saved_settings_are_what_create_uses() {
    let db = TestDb::world().await;
    let svc = db.service();
    save(
        &db,
        &svc,
        &SaveAlertSettings {
            default_cooldown_seconds: 600,
            max_active_per_customer: 1,
            default_lifetime_days: Some(7),
            observe_policy: ObservePolicy::AllFresh,
        },
    )
    .await
    .expect("saved");
    let usd = input(
        "USD",
        CustomerSide::CustomerBuy,
        AlertCondition::Above,
        "35",
    );
    let alert = db.create(&svc, CX1, &usd, &key()).await.expect("created");
    assert_eq!(alert.cooldown_seconds, 600);
    assert_eq!(alert.expires_at, Some(t0() + Duration::days(7)));
    let got = db.create(&svc, CX1, &usd, &key()).await;
    assert_eq!(
        refusal(got),
        ("validationFailed", Some("alertLimitReached"))
    );
    db.finish().await;
}
