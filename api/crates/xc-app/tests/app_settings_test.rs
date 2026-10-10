//! APP-4 (#994) — the organisation's draft lifetime (owner C4): `AppSettings::draft_expires_at` and
//! `AppSettingsService::save_in` (the command `appSettingsSave`).
//! Written before the implementation by the ticket author's delegate (claude/dell-claude-3). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design `docs/design/blueprint/customer-app.md` §4.1 `customer_app_settings`, §5 (the lifetime is
//! written only by `appSettingsSave` → `AppSettingsService::save` → `upsert`), §8 M1 (a later
//! organisation has no row until ADMIN saves one), §9.1 row APP-4. Ticket #994 §3 (`appSettingsSave`
//! → ADMIN, `Permission::SystemSettings`), §9 (by ADMIN: inserts, then updates one row; by a
//! non-ADMIN → `outOfScope`).
//!
//! Signatures this test assumes (the scaffold commit before this one):
//! - `AppSettings { organization_id: Uuid, planner_draft_ttl_seconds: Option<NonZeroU32> }`,
//!   `draft_expires_at(&self, created_at) -> Result<DateTime<Utc>, CustomerAppError>`.
//! - `AppSettingsService::new(pool)`; `save_in(&self, tx, actor: &xc_auth access Actor,
//!   ttl_seconds: u32, now) -> Result<AppSettings, CustomerAppError>` (async), which writes the
//!   row of `actor.organization_id` with `updated_by = actor.user_id`.

mod common;

use std::num::NonZeroU32;

use common::*;
use xc_auth::domain::access::role::Role;

#[test]
fn draft_expires_at_is_created_at_plus_the_lifetime() {
    let settings = AppSettings {
        organization_id: org(),
        planner_draft_ttl_seconds: NonZeroU32::new(604_800),
    };

    assert_eq!(settings.draft_expires_at(at(0)), Ok(at(604_800)));
    assert_eq!(settings.draft_expires_at(at(42)), Ok(at(42 + 604_800)));
}

#[test]
fn draft_expires_at_without_a_lifetime_is_app_settings_missing() {
    let settings = AppSettings {
        organization_id: org(),
        planner_draft_ttl_seconds: None,
    };

    assert_eq!(
        settings.draft_expires_at(at(0)),
        Err(CustomerAppError::AppSettingsMissing)
    );
}

#[test]
fn every_refusal_answers_its_v3_code() {
    assert_eq!(
        CustomerAppError::AppSettingsMissing.code(),
        "appSettingsMissing"
    );
    assert_eq!(CustomerAppError::StaleVersion.code(), "staleVersion");
    assert_eq!(CustomerAppError::OutOfScope.code(), "outOfScope");
    assert_eq!(CustomerAppError::NotFound.code(), "notFound");
    assert_eq!(
        CustomerAppError::RateUnavailable("x".into()).code(),
        "rateUnavailable"
    );
    assert_eq!(
        CustomerAppError::ValidationFailed { field: "branch" }.code(),
        "validationFailed"
    );
    assert_eq!(CustomerAppError::Server("x".into()).code(), "serverError");
}

#[tokio::test]
async fn admin_save_on_an_organisation_with_no_row_inserts_one() {
    let db = TestDb::start().await;
    db.delete_settings().await;
    let service = AppSettingsService::new(db.pool.clone());

    let mut tx = db.begin().await;
    let saved = service
        .save_in(&mut tx, &actor("admin-1", Role::Admin), 86_400, at(0))
        .await
        .expect("ADMIN saves the lifetime");
    tx.commit().await.expect("commit");

    assert_eq!(saved.organization_id, org());
    assert_eq!(saved.planner_draft_ttl_seconds, NonZeroU32::new(86_400));
    assert_eq!(
        db.settings_row().await,
        (1, Some(86_400), Some("admin-1".to_owned()))
    );
    db.finish().await;
}

#[tokio::test]
async fn a_second_admin_save_updates_the_same_row() {
    let db = TestDb::start().await;
    db.delete_settings().await;
    let service = AppSettingsService::new(db.pool.clone());
    let mut tx = db.begin().await;
    service
        .save_in(&mut tx, &actor("admin-1", Role::Admin), 86_400, at(0))
        .await
        .expect("first save");
    tx.commit().await.expect("commit");

    let mut tx = db.begin().await;
    service
        .save_in(&mut tx, &actor("admin-2", Role::Admin), 3_600, at(10))
        .await
        .expect("second save");
    tx.commit().await.expect("commit");

    assert_eq!(
        db.settings_row().await,
        (1, Some(3_600), Some("admin-2".to_owned()))
    );
    db.finish().await;
}

#[tokio::test]
async fn admin_save_replaces_the_seeded_seven_days() {
    let db = TestDb::start().await;
    assert_eq!(
        db.settings_row().await.1,
        Some(604_800),
        "APP-1 seeds 7 days"
    );
    let service = AppSettingsService::new(db.pool.clone());

    let mut tx = db.begin().await;
    service
        .save_in(&mut tx, &actor("admin-1", Role::Admin), 1_209_600, at(0))
        .await
        .expect("saved");
    tx.commit().await.expect("commit");

    assert_eq!(
        db.settings_row().await,
        (1, Some(1_209_600), Some("admin-1".to_owned()))
    );
    db.finish().await;
}

#[tokio::test]
async fn a_saved_lifetime_is_the_one_the_next_draft_gets() {
    let db = TestDb::start().await;
    let settings = AppSettingsService::new(db.pool.clone());
    let planner = PlannerService::new(db.pool.clone());
    let mut tx = db.begin().await;
    settings
        .save_in(&mut tx, &actor("admin-1", Role::Admin), 120, at(0))
        .await
        .expect("saved");
    tx.commit().await.expect("commit");

    let mut tx = db.begin().await;
    let draft = planner
        .save_in(
            &mut tx,
            org(),
            &session(),
            create(plan(
                "USD",
                xc_kernel::money::CustomerSide::CustomerBuy,
                "1",
            )),
            at(1_000),
        )
        .await
        .expect("a draft");
    tx.commit().await.expect("commit");

    assert_eq!(draft.expires_at, at(1_120));
    db.finish().await;
}

#[tokio::test]
async fn a_non_admin_save_is_out_of_scope_and_writes_nothing() {
    let db = TestDb::start().await;
    let service = AppSettingsService::new(db.pool.clone());

    for role in [
        Role::BranchManager,
        Role::Teller,
        Role::Finance,
        Role::Customer,
    ] {
        let mut tx = db.begin().await;
        let refused = service
            .save_in(&mut tx, &actor("not-admin", role), 60, at(0))
            .await
            .expect_err("not ADMIN");
        tx.commit().await.expect("commit after the refusal");
        assert_eq!(refused.code(), "outOfScope", "{role:?}");
    }
    assert_eq!(
        db.settings_row().await,
        (1, Some(604_800), Some("migration".to_owned()))
    );
    db.finish().await;
}

#[tokio::test]
async fn a_zero_lifetime_is_validation_failed_and_writes_nothing() {
    let db = TestDb::start().await;
    let service = AppSettingsService::new(db.pool.clone());

    let mut tx = db.begin().await;
    let refused = service
        .save_in(&mut tx, &actor("admin-1", Role::Admin), 0, at(0))
        .await
        .expect_err("0 s is not a lifetime");
    tx.commit().await.expect("commit after the refusal");

    assert_eq!(
        refused,
        CustomerAppError::ValidationFailed {
            field: "plannerDraftTtlSeconds"
        }
    );
    assert_eq!(
        db.settings_row().await,
        (1, Some(604_800), Some("migration".to_owned()))
    );
    db.finish().await;
}
