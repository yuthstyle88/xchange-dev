//! K6/P13 (feat/be-gaps-b; design `docs/design/blueprint/customer-identity-consent.md` §4.1, §4.3 "the purge job
//! deletes nothing for a data type with `period_days IS NULL`", `user-management-rbac.md` §9 as amended on this
//! branch): the retention purge and the expired-session purge run on a timer.
//!
//! Written before the implementation by the sub-controller (claude/2ef7d5a6). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! [required] by these tests:
//! - `xc_auth::infrastructure::expired_sessions::PgExpiredSessions::{new(PgPool), purge_before(now) -> AppResult<u64>}`
//!   deletes the `user_sessions` rows with `expires < now`.
//! - `xchange::bootstrap::purge_timers::{retention_job, purge_round, run, PurgeRound}`:
//!   - `purge_round(&RetentionJob, &PgExpiredSessions, now) -> PurgeRound { retention, sessions }` runs both purges,
//!     each even when the other fails;
//!   - `run(pool, every, stop: watch::Receiver<bool>)` runs a round at once, then every `every`, and returns when
//!     `stop` holds `true` or its sender is dropped.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

mod common;

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::infrastructure::expired_sessions::PgExpiredSessions;
use xc_customer::application::retention_job::RetentionJob;
use xc_customer::domain::retention::DataType;
use xc_kernel::pii::PiiKeys;
use xchange::bootstrap::purge_timers;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn org() -> Uuid {
    Uuid::parse_str(ORG).unwrap()
}

struct Db {
    pool: PgPool,
    user: Uuid,
}

impl Db {
    async fn start() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("k6tim");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("migrations")
                .as_path(),
        )
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("apply api/migrations");
        let _ = xc_customer::install_pii_keys(Arc::new(PiiKeys::parse(TEST_KEYS).unwrap()));
        sqlx::query(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("insert MAIN");
        let user = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES ($1, $2, 'U', 'ACTIVE', 'test')",
        )
        .bind(user)
        .bind(org())
        .execute(&pool)
        .await
        .expect("a user");
        Db { pool, user }
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn session(&self, expires: DateTime<Utc>) -> String {
        let hash = unique("h");
        sqlx::query(
            "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id) VALUES ($1, $2, 1, $3, $4)",
        )
        .bind(&hash)
        .bind(self.user)
        .bind(expires)
        .bind(org())
        .execute(&self.pool)
        .await
        .expect("a session");
        hash
    }

    async fn has_session(&self, hash: &str) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM user_sessions WHERE hash = $1")
            .bind(hash)
            .fetch_one(&self.pool)
            .await
            .expect("count a session")
            == 1
    }

    /// One row of every kind of customer data a retention period could delete, all long past any period.
    async fn old_customer_data(&self) {
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('CX-OLD', '{ORG}', 'CX-OLD', 'Old', 'LIVE', now() - interval '10 years')"
        ))
        .await;
        for kind in ["PASSPORT", "NATIONAL_ID", "PHONE", "EMAIL"] {
            self.exec(&format!(
                "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
                    lookup_hash, status, created_at, updated_at) \
                 VALUES (gen_random_uuid(), '{ORG}', 'CX-OLD', '{kind}', '\\x00', 'test-1', \
                    decode(md5(random()::text), 'hex'), 'REVOKED', now() - interval '10 years', now() - interval '10 years')"
            ))
            .await;
        }
        self.exec(&format!(
            "INSERT INTO planner_drafts (id, organization_id, customer_id, currency_code, customer_side, quantity, \
                expires_at, created_at) \
             VALUES (gen_random_uuid(), '{ORG}', 'CX-OLD', 'USD', 'CUSTOMER_BUY', 100, \
                now() - interval '9 years', now() - interval '10 years')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO assistant_conversations (id, organization_id, customer_id, locale, expires_at, created_at) \
             VALUES (gen_random_uuid(), '{ORG}', 'CX-OLD', 'th', now() - interval '9 years', now() - interval '10 years')"
        ))
        .await;
    }

    async fn customer_rows(&self) -> i64 {
        self.count("SELECT (SELECT count(*) FROM customer_identities) + (SELECT count(*) FROM planner_drafts) \
                    + (SELECT count(*) FROM assistant_conversations)")
            .await
    }

    async fn set_period(&self, data_type: DataType, days: Option<i32>) {
        RetentionJob::pg(self.pool.clone())
            .set_period(ORG, None, data_type, days, Utc::now())
            .await
            .expect("set a retention period");
    }
}

#[tokio::test]
async fn k6_1_a_round_deletes_sessions_expired_before_now_and_keeps_the_rest() {
    let db = Db::start().await;
    let now = Utc::now();
    let expired = db.session(now - chrono::Duration::seconds(1)).await;
    let long_expired = db.session(now - chrono::Duration::days(400)).await;
    let at_now = db.session(now).await;
    let live = db.session(now + chrono::Duration::hours(8)).await;

    let round = purge_timers::purge_round(
        &purge_timers::retention_job(db.pool.clone()),
        &PgExpiredSessions::new(db.pool.clone()),
        now,
    )
    .await;

    assert_eq!(round.sessions.expect("the session purge"), 2);
    assert!(!db.has_session(&expired).await);
    assert!(!db.has_session(&long_expired).await);
    assert!(
        db.has_session(&at_now).await,
        "expires exactly now: kept (strictly before)"
    );
    assert!(db.has_session(&live).await);
}

#[tokio::test]
async fn k6_2_no_period_set_deletes_no_customer_data() {
    let db = Db::start().await;
    db.old_customer_data().await;
    let before = db.customer_rows().await;
    assert_eq!(before, 6, "the seeded rows");

    // No retention_policies row at all.
    let round = purge_timers::purge_round(
        &purge_timers::retention_job(db.pool.clone()),
        &PgExpiredSessions::new(db.pool.clone()),
        Utc::now(),
    )
    .await;
    assert_eq!(round.retention.expect("the retention purge"), 0);
    assert_eq!(
        db.customer_rows().await,
        before,
        "nothing deleted without a period"
    );

    // A row for every type, each with no period (NULL).
    for t in [
        DataType::IdentityValue,
        DataType::Contact,
        DataType::ClosedCustomer,
        DataType::RateAlert,
        DataType::AssistantConversation,
        DataType::PlannerDraft,
    ] {
        db.set_period(t, None).await;
    }
    let round = purge_timers::purge_round(
        &purge_timers::retention_job(db.pool.clone()),
        &PgExpiredSessions::new(db.pool.clone()),
        Utc::now(),
    )
    .await;
    assert_eq!(round.retention.expect("the retention purge"), 0);
    assert_eq!(
        db.customer_rows().await,
        before,
        "a NULL period deletes nothing"
    );
}

#[tokio::test]
async fn k6_3_a_round_runs_the_retention_purge_for_a_period_that_is_set() {
    let db = Db::start().await;
    db.old_customer_data().await;
    db.set_period(DataType::IdentityValue, Some(30)).await;

    let round = purge_timers::purge_round(
        &purge_timers::retention_job(db.pool.clone()),
        &PgExpiredSessions::new(db.pool.clone()),
        Utc::now(),
    )
    .await;
    assert_eq!(
        round.retention.expect("the retention purge"),
        2,
        "the revoked PASSPORT and NATIONAL_ID"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM customer_identities WHERE type IN ('PHONE', 'EMAIL')")
            .await,
        2,
        "CONTACT has no period: kept"
    );
}

#[tokio::test]
async fn k6_4_a_failing_retention_purge_does_not_stop_the_session_purge() {
    let db = Db::start().await;
    let expired = db.session(Utc::now() - chrono::Duration::hours(1)).await;
    db.exec("ALTER TABLE retention_policies RENAME TO retention_policies_gone")
        .await;

    let round = purge_timers::purge_round(
        &purge_timers::retention_job(db.pool.clone()),
        &PgExpiredSessions::new(db.pool.clone()),
        Utc::now(),
    )
    .await;
    assert!(
        round.retention.is_err(),
        "the retention purge failed: {round:?}"
    );
    assert_eq!(round.sessions.as_ref().ok(), Some(&1), "{round:?}");
    assert!(!db.has_session(&expired).await);
}

#[tokio::test]
async fn k6_5_run_purges_at_once_then_every_interval_and_stops_when_told() {
    let db = Db::start().await;
    let first = db.session(Utc::now() - chrono::Duration::hours(1)).await;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(purge_timers::run(
        db.pool.clone(),
        Duration::from_millis(100),
        stop_rx,
    ));

    wait_until_gone(&db, &first).await;
    let second = db.session(Utc::now() - chrono::Duration::seconds(1)).await;
    wait_until_gone(&db, &second).await;

    stop_tx.send(true).expect("send stop");
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("run returns once told to stop")
        .expect("run did not panic");
}

#[tokio::test]
async fn k6_6_run_stops_when_the_stop_sender_is_dropped() {
    let db = Db::start().await;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(purge_timers::run(
        db.pool.clone(),
        Duration::from_millis(100),
        stop_rx,
    ));
    tokio::time::sleep(Duration::from_millis(250)).await;
    drop(stop_tx);
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("run returns once the sender is gone")
        .expect("run did not panic");
}

async fn wait_until_gone(db: &Db, hash: &str) {
    for _ in 0..100 {
        if !db.has_session(hash).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("session {hash} was not purged within 5 s");
}
