//! AL-2 (#1002, rate alerts RA-2 of #561) — `RateAlertService::create` (design §2 R1, R2, R5, R11, R14; §4.3;
//! §9.1 row "RA-2"; ticket §8 `rateAlertCreate`, §9 edge cases).
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.
//!
//! Signature this test assumes (ticket §7, §8):
//! `create(&self, tx: &mut Transaction<'_, Postgres>, customer_id: &str, input: &CreateRateAlert,
//!  idempotency_key: &str, now: DateTime<Utc>) -> Result<RateAlert, RateAlertError>`; the future is `Send`.
//! The organisation (for the settings row) is the customer's. `now` is the clock for the default expiry and for
//! "active" in the maximum. The idempotency of the service is kept on `UNIQUE (customer_id, idempotency_key)`
//! (§4.3): the same key and the same input give the first alert back; another input under the key is
//! `idempotencyConflict`.

mod common;

use std::sync::Arc;

use chrono::Duration;
use common::*;
use tokio::sync::Barrier;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Side, UnitSize};

fn usd_above(target: &str) -> CreateRateAlert {
    input(
        "USD",
        CustomerSide::CustomerBuy,
        AlertCondition::Above,
        target,
    )
}

// ── R1: the side ────────────────────────────────────────────────────────────

#[tokio::test]
async fn customer_buy_is_stored_as_the_shop_selling() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = db
        .create(&svc, CX1, &usd_above("35"), &key())
        .await
        .expect("the alert is created");
    assert_eq!(alert.shop_side, Side::Sell);
    assert_eq!(
        db.row(alert.id).await.shop_side,
        "SELL",
        "CUSTOMER_BUY stores SELL (R1)"
    );
    db.finish().await;
}

#[tokio::test]
async fn customer_sell_is_stored_as_the_shop_buying() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = db
        .create(
            &svc,
            CX1,
            &input(
                "USD",
                CustomerSide::CustomerSell,
                AlertCondition::Above,
                "35",
            ),
            &key(),
        )
        .await
        .expect("the alert is created");
    assert_eq!(alert.shop_side, Side::Buy);
    assert_eq!(
        db.row(alert.id).await.shop_side,
        "BUY",
        "CUSTOMER_SELL stores BUY (R1)"
    );
    db.finish().await;
}

// ── R2: the unit ────────────────────────────────────────────────────────────

#[tokio::test]
async fn unit_size_is_the_registers() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    for (code, unit) in [("JPY", "100"), ("KRW", "1000"), ("USD", "1")] {
        let alert = db
            .create(
                &svc,
                CX1,
                &input(code, CustomerSide::CustomerBuy, AlertCondition::Above, "99"),
                &key(),
            )
            .await
            .unwrap_or_else(|e| panic!("{code}: the alert is created: {e:?}"));
        assert_eq!(
            alert.unit_size,
            UnitSize::parse(unit).expect("a unit"),
            "{code}"
        );
        assert_eq!(
            db.row(alert.id).await.unit_size,
            dec(unit),
            "{code}: unit_size stored from the register"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn the_row_holds_the_input_the_customer_and_the_organisation() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let k = key();
    let alert = db
        .create(
            &svc,
            CX2,
            &input(
                "USD",
                CustomerSide::CustomerBuy,
                AlertCondition::Below,
                "33.12345678",
            ),
            &k,
        )
        .await
        .expect("the alert is created");
    let row = db.row(alert.id).await;
    assert_eq!(row.customer_id, CX2);
    assert_eq!(row.organization_id, org());
    assert_eq!(row.branch_id, MAIN);
    assert_eq!(row.currency_code, "USD");
    assert_eq!(row.condition, "BELOW");
    assert_eq!(row.target, dec("33.12345678"));
    assert!(row.enabled);
    assert_eq!(row.deleted_at, None);
    assert_eq!(row.version, 1);
    assert_eq!(row.idempotency_key, k);
    assert_eq!(alert.version, 1);
    assert_eq!(alert.customer_id, CX2);
    db.finish().await;
}

// ── R14: settings and the maximum ───────────────────────────────────────────

#[tokio::test]
async fn without_a_settings_row_the_create_is_refused_alert_settings_missing() {
    let db = TestDb::world().await;
    let svc = db.service();
    let got = db.create(&svc, CX1, &usd_above("35"), &key()).await;
    assert_eq!(
        refusal(got),
        ("validationFailed", Some("alertSettingsMissing"))
    );
    assert_eq!(db.alerts_of(CX1).await, 0, "a refusal writes nothing");
    db.finish().await;
}

#[tokio::test]
async fn cooldown_is_copied_from_the_settings_at_creation() {
    let db = TestDb::world().await;
    db.settings(3600, 10, Some(30), "ALL_FRESH").await;
    let svc = db.service();
    let alert = db
        .create(&svc, CX1, &usd_above("35"), &key())
        .await
        .expect("created");
    assert_eq!(alert.cooldown_seconds, 3600);
    db.settings(7200, 10, Some(30), "ALL_FRESH").await;
    assert_eq!(
        db.row(alert.id).await.cooldown_seconds,
        3600,
        "a later settings change leaves an existing alert's cooldown as it was copied"
    );
    db.finish().await;
}

#[tokio::test]
async fn at_the_maximum_the_create_is_refused_and_paused_alerts_count() {
    let db = TestDb::world().await;
    db.settings(86400, 2, Some(30), "ALL_FRESH").await;
    let svc = db.service();
    db.create(&svc, CX1, &usd_above("35"), &key())
        .await
        .expect("first");
    db.insert_alert(CX1, MAIN, &[("enabled", "false")]).await;

    let got = db.create(&svc, CX1, &usd_above("36"), &key()).await;
    assert_eq!(
        refusal(got),
        ("validationFailed", Some("alertLimitReached")),
        "one enabled + one paused = 2 = the maximum (Q-R2: paused included)"
    );
    assert_eq!(db.alerts_of(CX1).await, 2);

    db.create(&svc, CX2, &usd_above("35"), &key())
        .await
        .expect("the maximum is per customer");
    db.finish().await;
}

#[tokio::test]
async fn expired_and_deleted_alerts_do_not_count_toward_the_maximum() {
    let db = TestDb::world().await;
    db.settings(86400, 1, Some(30), "ALL_FRESH").await;
    let svc = db.service();
    // Expired and deleted under any clock: "active" is undeleted and unexpired, paused included (§4.1).
    db.insert_alert(CX1, MAIN, &[("expires_at", "'2020-01-01T00:00:00Z'")])
        .await;
    db.insert_alert(CX1, MAIN, &[("deleted_at", "'2020-01-01T00:00:00Z'")])
        .await;
    db.create(&svc, CX1, &usd_above("35"), &key())
        .await
        .expect("an expired and a deleted alert are not active");
    db.finish().await;
}

#[tokio::test]
async fn two_concurrent_creates_at_one_below_the_maximum_give_exactly_one_success() {
    let db = TestDb::world().await;
    db.settings(86400, 2, Some(30), "ALL_FRESH").await;
    let svc = db.service();
    db.create(&svc, CX1, &usd_above("35"), &key())
        .await
        .expect("max − 1");

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for target in ["36", "37"] {
        let svc = svc.clone();
        let barrier = barrier.clone();
        let pool = db.pool.clone();
        tasks.push(tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("begin");
            barrier.wait().await;
            let result = svc
                .create(&mut tx, CX1, &usd_above(target), &key(), t0())
                .await;
            // Hold the transaction open so that, without the customer-row lock, both would count 1.
            tokio::time::sleep(HOLD).await;
            settle(tx, result)
                .await
                .map(|_| ())
                .map_err(|e| (e.code(), e.reason()))
        }));
    }
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(
            within(MUST_FINISH, "both creates finish", task)
                .await
                .expect("the task does not panic"),
        );
    }
    outcomes.sort();
    assert_eq!(
        outcomes,
        vec![Ok(()), Err(("validationFailed", Some("alertLimitReached")))],
        "the count is taken under customers … FOR UPDATE (R14)"
    );
    assert_eq!(db.alerts_of(CX1).await, 2);
    db.finish().await;
}

// ── idempotency ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_same_key_with_the_same_body_gives_one_alert() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let k = key();
    let first = db
        .create(&svc, CX1, &usd_above("35"), &k)
        .await
        .expect("first");
    let again = db
        .create(&svc, CX1, &usd_above("35"), &k)
        .await
        .expect("a replay is not a refusal");
    assert_eq!(again.id, first.id, "the replay answers the first alert");
    assert_eq!(db.alerts_of(CX1).await, 1);
    db.finish().await;
}

#[tokio::test]
async fn the_same_key_with_another_body_is_idempotency_conflict() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let k = key();
    db.create(&svc, CX1, &usd_above("35"), &k)
        .await
        .expect("first");
    let got = db.create(&svc, CX1, &usd_above("36"), &k).await;
    assert_eq!(refusal(got).0, "idempotencyConflict");
    let got = db
        .create(
            &svc,
            CX1,
            &input(
                "USD",
                CustomerSide::CustomerSell,
                AlertCondition::Above,
                "35",
            ),
            &k,
        )
        .await;
    assert_eq!(refusal(got).0, "idempotencyConflict");
    assert_eq!(db.alerts_of(CX1).await, 1);
    db.finish().await;
}

#[tokio::test]
async fn one_key_used_by_two_customers_gives_two_alerts() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let k = key();
    let a = db
        .create(&svc, CX1, &usd_above("35"), &k)
        .await
        .expect("CX-1");
    let b = db
        .create(&svc, CX2, &usd_above("35"), &k)
        .await
        .expect("CX-2");
    assert_ne!(
        a.id, b.id,
        "keys are per customer: UNIQUE (customer_id, idempotency_key)"
    );
    db.finish().await;
}

// ── R11: expiry ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn without_expires_at_the_alert_lives_the_default_lifetime() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let alert = db
        .create(&svc, CX1, &usd_above("35"), &key())
        .await
        .expect("created");
    let expected = t0() + Duration::days(30);
    assert_eq!(alert.expires_at, Some(expected));
    assert_eq!(
        db.row(alert.id).await.expires_at,
        Some(expected),
        "now + default_lifetime_days (Q-R4)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_chosen_expires_at_is_kept() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let chosen = t0() + Duration::days(90);
    let alert = db
        .create(
            &svc,
            CX1,
            &CreateRateAlert {
                expires_at: Some(chosen),
                ..usd_above("35")
            },
            &key(),
        )
        .await
        .expect("created");
    assert_eq!(db.row(alert.id).await.expires_at, Some(chosen));
    db.finish().await;
}

#[tokio::test]
async fn a_null_default_lifetime_means_no_expiry() {
    let db = TestDb::world().await;
    db.settings(86400, 10, None, "ALL_FRESH").await;
    let svc = db.service();
    let alert = db
        .create(&svc, CX1, &usd_above("35"), &key())
        .await
        .expect("created");
    assert_eq!(db.row(alert.id).await.expires_at, None);
    db.finish().await;
}

// ── validation ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_currency_the_branch_does_not_publish_is_refused() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    let got = db
        .create(
            &svc,
            CX1,
            &input(
                "EUR",
                CustomerSide::CustomerBuy,
                AlertCondition::Above,
                "40",
            ),
            &key(),
        )
        .await;
    assert_eq!(
        refusal(got).0,
        "validationFailed",
        "MAIN's current publication has no EUR line"
    );

    let got = db
        .create(
            &svc,
            CX1,
            &CreateRateAlert {
                branch: BranchId::new(NORTH),
                ..usd_above("35")
            },
            &key(),
        )
        .await;
    assert_eq!(
        refusal(got).0,
        "validationFailed",
        "NORTH has never published"
    );
    assert_eq!(db.alerts_of(CX1).await, 0);
    db.finish().await;
}

#[tokio::test]
async fn a_target_that_is_not_positive_or_has_more_than_eight_places_is_refused() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    for bad in ["0", "-1", "34.123456789"] {
        let got = db.create(&svc, CX1, &usd_above(bad), &key()).await;
        assert_eq!(refusal(got).0, "validationFailed", "target {bad}");
    }
    assert_eq!(db.alerts_of(CX1).await, 0);
    db.create(&svc, CX1, &usd_above("34.12345678"), &key())
        .await
        .expect("eight places are accepted");
    db.finish().await;
}

// ── R5: the first observation never fires ───────────────────────────────────

#[tokio::test]
async fn create_observes_the_current_publication_without_firing() {
    let db = TestDb::world().await;
    db.owners_settings().await;
    let svc = db.service();
    // MAIN v1 USD sell = 34.00 (the shop side of CUSTOMER_BUY).
    let at_target = db
        .create(&svc, CX1, &usd_above("34"), &key())
        .await
        .expect("created");
    assert_eq!(at_target.relation, Relation::Satisfied);
    let row = db.row(at_target.id).await;
    assert_eq!(
        (row.relation.as_str(), row.episode_no),
        ("SATISFIED", 0),
        "already past the target at creation: SATISFIED, no episode (R5)"
    );

    let below = db
        .create(
            &svc,
            CX1,
            &input(
                "USD",
                CustomerSide::CustomerBuy,
                AlertCondition::Below,
                "30",
            ),
            &key(),
        )
        .await
        .expect("created");
    assert_eq!(db.row(below.id).await.relation, "NOT_SATISFIED");
    assert_eq!(db.events().await, 0, "creating never writes an event");
    db.finish().await;
}

#[tokio::test]
async fn create_on_a_stale_side_leaves_the_relation_unknown() {
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
    let alert = db
        .create(&svc, CX1, &usd_above("34"), &key())
        .await
        .expect("a stale side does not stop the create");
    assert_eq!(
        db.row(alert.id).await.relation,
        "UNKNOWN",
        "ALL_FRESH skips a stale side and a skipped side sets no relation (R8)"
    );
    assert_eq!(db.events().await, 0);
    db.finish().await;
}
