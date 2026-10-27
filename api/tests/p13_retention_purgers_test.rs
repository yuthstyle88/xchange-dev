//! P13 + rate-alerts retention (feat/be-gaps-b; design `docs/design/blueprint/customer-app.md` P13, the
//! `planner_drafts` state line; `rate-alerts.md` "a deleted or expired alert and its events use data type
//! `RATE_ALERT`"; `customer-identity-consent.md` §4.1, §4.3): `PLANNER_DRAFT` and `RATE_ALERT` are purged through the
//! `RetentionPurger` port, and an expired draft is gone from every read.
//!
//! Written before the implementation by the sub-controller (claude/2ef7d5a6). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! [required] by these tests:
//! - `RetentionJob::purge` calls, for each policy row with a period whose type is not one of its own three, the
//!   registered `RetentionPurger` of that type with (org, cutoff = now − period, now), and adds what it answers. A
//!   type with a period and no registered purger is skipped (no error, nothing deleted).
//! - `PgPlannerDraftPurger`: deletes the org's `planner_drafts` with `expires_at <= now` or `created_at < cutoff`;
//!   answers the drafts deleted. Another organisation's rows are never touched.
//! - `PgRateAlertPurger`: deletes the org's `rate_alerts` with `deleted_at < cutoff` or `expires_at < cutoff`, and
//!   their `rate_alert_events` first; answers the alerts deleted. A live alert (not deleted, not expired), or one
//!   only disabled (`UNIT_CHANGED`), is kept whatever its age.
//! - `PgPlannerRepository::{list, get_in}` answer no draft whose `expires_at <= now()`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

mod common;

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_alert::infrastructure::rate_alert_retention::PgRateAlertPurger;
use xc_app::domain::app_session::SessionHash;
use xc_app::domain::owner::DraftOwner;
use xc_app::domain::planner::repository::PlannerRepository;
use xc_app::infrastructure::planner_repository::PgPlannerRepository;
use xc_app::infrastructure::planner_retention::PgPlannerDraftPurger;
use xc_customer::application::retention_job::{RetentionJob, RetentionPurger};
use xc_customer::domain::retention::DataType;
use xchange::bootstrap::purge_timers;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const ORG2: &str = "00000000-0000-0000-0000-0000000000b2";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

struct Db {
    pool: PgPool,
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
        let schema = unique("p13ret");
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
        let db = Db { pool };
        db.exec(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
        )
        .await;
        db.exec(&format!(
            "INSERT INTO organizations (id, code, timezone, created_at) VALUES ('{ORG2}', 'O2', 'Asia/Bangkok', now())"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('O2B', 'O2B', '{ORG2}')"
        ))
        .await;
        for (id, org) in [("CX-1", ORG), ("CX-2", ORG2)] {
            db.exec(&format!(
                "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
                 VALUES ('{id}', '{org}', '{id}', 'C', 'LIVE', now() - interval '5 years')"
            ))
            .await;
        }
        db
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn exists(&self, table: &str, id: Uuid) -> bool {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE id = $1"
        )))
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("count")
            == 1
    }

    async fn set_period(&self, org: &str, data_type: DataType, days: Option<i32>) {
        RetentionJob::pg(self.pool.clone())
            .set_period(org, None, data_type, days, Utc::now())
            .await
            .expect("set a retention period");
    }

    /// A customer-owned draft of `org`, created `created` and expiring `expires` (SQL expressions).
    async fn draft(&self, org: &str, customer: &str, created: &str, expires: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO planner_drafts (id, organization_id, customer_id, currency_code, customer_side, quantity, \
                expires_at, created_at, updated_at) \
             VALUES ('{id}', '{org}', '{customer}', 'USD', 'CUSTOMER_BUY', 100, {expires}, {created}, {created})"
        ))
        .await;
        id
    }

    async fn session_draft(&self, hash: &str, expires: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.exec(&format!(
            "INSERT INTO planner_drafts (id, organization_id, session_hash, currency_code, customer_side, budget_thb, \
                expires_at) \
             VALUES ('{id}', '{ORG}', '{hash}', 'USD', 'CUSTOMER_SELL', 1000, {expires})"
        ))
        .await;
        id
    }

    /// An alert of `org`'s customer `customer` at that organisation's branch, created `created`.
    async fn alert(
        &self,
        org: &str,
        customer: &str,
        created: &str,
        deleted: &str,
        expires: &str,
        disabled: &str,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let branch = if org == ORG { "'MAIN'" } else { "'O2B'" };
        let enabled = if deleted == "NULL" && disabled == "NULL" {
            "true"
        } else {
            "false"
        };
        self.exec(&format!(
            "INSERT INTO rate_alerts (id, organization_id, customer_id, branch_id, currency_code, shop_side, unit_size, \
                condition, target, enabled, cooldown_seconds, expires_at, relation, episode_no, disabled_reason, \
                deleted_at, version, idempotency_key, created_at, updated_at) \
             VALUES ('{id}', '{org}', '{customer}', {branch}, 'USD', 'BUY', 1, 'ABOVE', 35, {enabled}, 0, {expires}, \
                'UNKNOWN', 0, {disabled}, {deleted}, 1, '{key}', {created}, {created})",
            key = unique("k")
        ))
        .await;
        id
    }

    /// One `rate_alert_events` row of `alert` on a fresh MAIN publication.
    async fn event(&self, alert: Uuid) -> Uuid {
        let approval: Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', 'MAIN', $1, 'h', 'p', '{}', 'test', now(), 'APPROVED', $1, $2) \
             RETURNING id",
        )
        .bind(unique("pub"))
        .bind(Uuid::parse_str(ORG).unwrap())
        .fetch_one(&self.pool)
        .await
        .expect("approved request");
        let version: i32 =
            sqlx::query_scalar("SELECT COALESCE(max(version), 0) + 1 FROM rate_publications")
                .fetch_one(&self.pool)
                .await
                .expect("next version");
        let publication: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), 'MAIN', $1, now(), $2, 'test') RETURNING id",
        )
        .bind(version)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("publication");
        let line: Uuid = sqlx::query_scalar(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, \
             source_snapshot) VALUES (gen_random_uuid(), $1, 'USD', 1, 36, 36.5, '{}') RETURNING id",
        )
        .bind(publication)
        .fetch_one(&self.pool)
        .await
        .expect("line");
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO rate_alert_events (id, organization_id, alert_id, published_rate_line_id, observed_rate, \
             trigger_at, episode_no, created_at) VALUES ($1, $2, $3, $4, 36, now(), 1, now())",
        )
        .bind(id)
        .bind(Uuid::parse_str(ORG).unwrap())
        .bind(alert)
        .bind(line)
        .execute(&self.pool)
        .await
        .expect("an event");
        id
    }
}

// ── PLANNER_DRAFT ────────────────────────────────────────────────────────────

#[tokio::test]
async fn p13_1_planner_draft_purge_deletes_expired_and_too_old_drafts_of_the_org_only() {
    let db = Db::start().await;
    let expired = db
        .draft(
            ORG,
            "CX-1",
            "now() - interval '8 days'",
            "now() - interval '1 day'",
        )
        .await;
    let too_old = db
        .draft(
            ORG,
            "CX-1",
            "now() - interval '40 days'",
            "now() + interval '1 day'",
        )
        .await;
    let live = db
        .draft(
            ORG,
            "CX-1",
            "now() - interval '1 day'",
            "now() + interval '6 days'",
        )
        .await;
    let other_org = db
        .draft(
            ORG2,
            "CX-2",
            "now() - interval '400 days'",
            "now() - interval '300 days'",
        )
        .await;
    db.set_period(ORG, DataType::PlannerDraft, Some(30)).await;

    let deleted = purge_timers::retention_job(db.pool.clone())
        .purge(Utc::now())
        .await
        .expect("purge");
    assert_eq!(
        deleted, 2,
        "the expired draft and the one older than the period"
    );
    assert!(!db.exists("planner_drafts", expired).await);
    assert!(!db.exists("planner_drafts", too_old).await);
    assert!(db.exists("planner_drafts", live).await);
    assert!(
        db.exists("planner_drafts", other_org).await,
        "ORG2 has no period"
    );
}

#[tokio::test]
async fn p13_2_the_planner_purger_alone_answers_its_type_and_its_count() {
    let db = Db::start().await;
    db.draft(
        ORG,
        "CX-1",
        "now() - interval '8 days'",
        "now() - interval '1 day'",
    )
    .await;
    let purger = PgPlannerDraftPurger::new(db.pool.clone());
    assert_eq!(purger.data_type(), DataType::PlannerDraft);
    let now = Utc::now();
    let answered = purger
        .purge(
            Uuid::parse_str(ORG).unwrap(),
            now - chrono::Duration::days(30),
            now,
        )
        .await
        .expect("purge");
    assert_eq!(answered, 1);
}

#[tokio::test]
async fn p13_3_without_a_period_an_expired_draft_stays_stored() {
    let db = Db::start().await;
    let expired = db
        .draft(
            ORG,
            "CX-1",
            "now() - interval '400 days'",
            "now() - interval '390 days'",
        )
        .await;
    db.set_period(ORG, DataType::PlannerDraft, None).await;
    let deleted = purge_timers::retention_job(db.pool.clone())
        .purge(Utc::now())
        .await
        .expect("purge");
    assert_eq!(deleted, 0);
    assert!(
        db.exists("planner_drafts", expired).await,
        "a type with no period is never purged"
    );
}

// ── RATE_ALERT ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn p13_4_rate_alert_purge_deletes_deleted_or_expired_alerts_past_the_period_with_their_events()
 {
    let db = Db::start().await;
    let old = "now() - interval '400 days'";
    let deleted_long_ago = db
        .alert(
            ORG,
            "CX-1",
            old,
            "now() - interval '40 days'",
            "NULL",
            "NULL",
        )
        .await;
    let deleted_event = db.event(deleted_long_ago).await;
    let expired_long_ago = db
        .alert(
            ORG,
            "CX-1",
            old,
            "NULL",
            "now() - interval '40 days'",
            "NULL",
        )
        .await;
    let deleted_recently = db
        .alert(
            ORG,
            "CX-1",
            old,
            "now() - interval '10 days'",
            "NULL",
            "NULL",
        )
        .await;
    let live_and_old = db.alert(ORG, "CX-1", old, "NULL", "NULL", "NULL").await;
    let live_event = db.event(live_and_old).await;
    let disabled_only = db
        .alert(ORG, "CX-1", old, "NULL", "NULL", "'UNIT_CHANGED'")
        .await;
    let other_org = db
        .alert(
            ORG2,
            "CX-2",
            old,
            "now() - interval '300 days'",
            "NULL",
            "NULL",
        )
        .await;
    db.set_period(ORG, DataType::RateAlert, Some(30)).await;

    let deleted = purge_timers::retention_job(db.pool.clone())
        .purge(Utc::now())
        .await
        .expect("purge");
    assert_eq!(
        deleted, 2,
        "the alert deleted 40 days ago and the one expired 40 days ago"
    );
    assert!(!db.exists("rate_alerts", deleted_long_ago).await);
    assert!(
        !db.exists("rate_alert_events", deleted_event).await,
        "its events go first"
    );
    assert!(!db.exists("rate_alerts", expired_long_ago).await);
    assert!(
        db.exists("rate_alerts", deleted_recently).await,
        "within the period"
    );
    assert!(
        db.exists("rate_alerts", live_and_old).await,
        "a live alert is kept whatever its age"
    );
    assert!(db.exists("rate_alert_events", live_event).await);
    assert!(
        db.exists("rate_alerts", disabled_only).await,
        "disabled is not ended"
    );
    assert!(
        db.exists("rate_alerts", other_org).await,
        "ORG2 has no period"
    );
}

#[tokio::test]
async fn p13_5_the_rate_alert_purger_alone_answers_its_type_and_without_a_period_nothing_goes() {
    let db = Db::start().await;
    let gone = db
        .alert(
            ORG,
            "CX-1",
            "now() - interval '400 days'",
            "now() - interval '300 days'",
            "NULL",
            "NULL",
        )
        .await;
    let purger = PgRateAlertPurger::new(db.pool.clone());
    assert_eq!(purger.data_type(), DataType::RateAlert);

    db.set_period(ORG, DataType::RateAlert, None).await;
    let deleted = purge_timers::retention_job(db.pool.clone())
        .purge(Utc::now())
        .await
        .expect("purge");
    assert_eq!(deleted, 0);
    assert!(db.exists("rate_alerts", gone).await, "no period: kept");

    let now = Utc::now();
    let answered = purger
        .purge(
            Uuid::parse_str(ORG).unwrap(),
            now - chrono::Duration::days(30),
            now,
        )
        .await
        .expect("purge");
    assert_eq!(answered, 1);
    assert!(!db.exists("rate_alerts", gone).await);
}

#[tokio::test]
async fn p13_6_a_period_for_a_type_with_no_registered_purger_is_skipped() {
    let db = Db::start().await;
    let draft = db
        .draft(
            ORG,
            "CX-1",
            "now() - interval '8 days'",
            "now() - interval '1 day'",
        )
        .await;
    db.set_period(ORG, DataType::PlannerDraft, Some(1)).await;
    let deleted = RetentionJob::pg(db.pool.clone())
        .purge(Utc::now())
        .await
        .expect("no purger registered for PLANNER_DRAFT: skipped, not an error");
    assert_eq!(deleted, 0);
    assert!(db.exists("planner_drafts", draft).await);
}

#[tokio::test]
async fn p13_7_the_retention_job_registers_both_purgers_through_the_port() {
    let db = Db::start().await;
    db.draft(
        ORG,
        "CX-1",
        "now() - interval '8 days'",
        "now() - interval '1 day'",
    )
    .await;
    db.alert(
        ORG,
        "CX-1",
        "now() - interval '400 days'",
        "now() - interval '300 days'",
        "NULL",
        "NULL",
    )
    .await;
    db.set_period(ORG, DataType::PlannerDraft, Some(30)).await;
    db.set_period(ORG, DataType::RateAlert, Some(30)).await;
    let job = RetentionJob::pg(db.pool.clone())
        .with_purger(Arc::new(PgPlannerDraftPurger::new(db.pool.clone())))
        .with_purger(Arc::new(PgRateAlertPurger::new(db.pool.clone())));
    assert_eq!(job.purge(Utc::now()).await.expect("purge"), 2);
}

// ── an expired draft is gone from reads ──────────────────────────────────────

#[tokio::test]
async fn p13_8_list_and_get_answer_no_expired_draft() {
    let db = Db::start().await;
    let live = db
        .draft(
            ORG,
            "CX-1",
            "now() - interval '1 day'",
            "now() + interval '6 days'",
        )
        .await;
    let expired = db
        .draft(
            ORG,
            "CX-1",
            "now() - interval '8 days'",
            "now() - interval '1 second'",
        )
        .await;
    let hash = unique("sess");
    let live_session = db.session_draft(&hash, "now() + interval '1 day'").await;
    let expired_session = db.session_draft(&hash, "now() - interval '1 minute'").await;
    let repo = PgPlannerRepository::new(db.pool.clone());

    let listed: Vec<Uuid> = repo
        .list(&DraftOwner::Customer("CX-1".to_owned()))
        .await
        .expect("list")
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(listed, vec![live], "only the live draft");

    let listed: Vec<Uuid> = repo
        .list(&DraftOwner::Session(SessionHash::from_stored(hash.clone())))
        .await
        .expect("list")
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(listed, vec![live_session], "only the live session draft");

    let mut tx = db.pool.begin().await.expect("begin");
    assert!(repo.get_in(&mut tx, live).await.expect("get").is_some());
    assert!(
        repo.get_in(&mut tx, expired).await.expect("get").is_none(),
        "an expired draft is gone"
    );
    assert!(
        repo.get_in(&mut tx, expired_session)
            .await
            .expect("get")
            .is_none()
    );
    tx.rollback().await.expect("rollback");
}
