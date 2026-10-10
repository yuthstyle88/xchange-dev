//! AL-1 (rate alerts RA-1, #561): migration M1 `api/migrations/<version>_rate_alerts.sql` (design
//! `docs/design/blueprint/rate-alerts.md` §4.1, §8 M1, §9.1 row "RA-1").
//!
//! Written before the implementation by the ticket author (claude/5edf7bb5). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.
//! Each test works in its own schema, so the tests run in parallel against one database.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

/// `main`'s newest migration when AL-1 was cut (`20261031100000_kiosk_quote_id.sql`).
const NEWEST_WHEN_CUT: i64 = 20261031100000;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const ALERT: &str = "a1000000-0000-0000-0000-000000000001";
const LINE: &str = "b1000000-0000-0000-0000-000000000001";
const LINE2: &str = "b1000000-0000-0000-0000-000000000002";

const CHECK_VIOLATION: &str = "23514";
const UNIQUE_VIOLATION: &str = "23505";
const FOREIGN_KEY_VIOLATION: &str = "23503";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

/// The version of the one file named `<version>_rate_alerts.sql`.
fn rate_alerts_version() -> i64 {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(migrations_dir()).expect("read api/migrations") {
        let name = entry.expect("a directory entry").file_name();
        let name = name.to_string_lossy().into_owned();
        if let Some(version) = name.strip_suffix("_rate_alerts.sql") {
            found.push(version.parse::<i64>().expect("a numeric migration version"));
        }
    }
    assert_eq!(
        found.len(),
        1,
        "exactly one api/migrations/<version>_rate_alerts.sql exists: {found:?}"
    );
    found[0]
}

struct Db {
    pool: PgPool,
}

impl Db {
    async fn fresh() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("al1");
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
            .max_connections(2)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        Db { pool }
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn text(&self, sql: &str) -> String {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// `Ok` or the SQLSTATE of the refusal (`"?"` when the error is not a database error).
    async fn exec(&self, sql: &str) -> Result<(), String> {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| match e.as_database_error() {
                Some(d) => format!("{} ({})", d.code().unwrap_or_default(), d.message()),
                None => format!("? ({e})"),
            })
    }

    async fn ok(&self, sql: &str, what: &str) {
        if let Err(e) = self.exec(sql).await {
            panic!("{what} is accepted: {e}\n{sql}");
        }
    }

    async fn refused(&self, sql: &str, sqlstate: &str, what: &str) {
        match self.exec(sql).await {
            Ok(()) => panic!("{what} is refused, but it was accepted\n{sql}"),
            Err(e) => assert!(
                e.starts_with(sqlstate),
                "{what} is refused with SQLSTATE {sqlstate}, got {e}\n{sql}"
            ),
        }
    }

    /// A branch, a customer, a staff user, and one publication of USD with two lines' worth of ids.
    async fn world(&self) {
        self.ok(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
            "the branch MAIN",
        )
        .await;
        self.ok(
            &format!(
                "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
                 VALUES ('CX-1', '{ORG}', 'CX-1', 'Somchai', 'LIVE', now()), \
                        ('CX-2', '{ORG}', 'CX-2', 'Somsri', 'LIVE', now())"
            ),
            "two customers",
        )
        .await;
        self.ok(
            &format!(
                "INSERT INTO users (id, organization_id, name, status, created_by) \
                 VALUES ('c1000000-0000-0000-0000-000000000001', '{ORG}', 'Exec', 'ACTIVE', 'test')"
            ),
            "an EXECUTIVE user",
        )
        .await;
        self.ok(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key) VALUES \
             ('d1000000-0000-0000-0000-000000000001', 'RATE_PUBLISH', 'MAIN', 'x', 'h', 'p', '{}', 'test', now(), \
              'APPROVED', 'k1'), \
             ('d1000000-0000-0000-0000-000000000002', 'RATE_PUBLISH', 'MAIN', 'y', 'h', 'p', '{}', 'test', now(), \
              'APPROVED', 'k2')",
            "two approvals",
        )
        .await;
        self.ok(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES ('e1000000-0000-0000-0000-000000000001', 'MAIN', 1, now(), \
                     'd1000000-0000-0000-0000-000000000001', 'test'), \
                    ('e1000000-0000-0000-0000-000000000002', 'MAIN', 2, now(), \
                     'd1000000-0000-0000-0000-000000000002', 'test')",
            "two publications",
        )
        .await;
        self.ok(
            &format!(
                "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, \
                 source_snapshot) VALUES \
                 ('{LINE}', 'e1000000-0000-0000-0000-000000000001', 'USD', 1, 33.5, 34.0, '{{}}'), \
                 ('{LINE2}', 'e1000000-0000-0000-0000-000000000002', 'USD', 1, 33.6, 34.1, '{{}}')"
            ),
            "two published USD lines",
        )
        .await;
    }

    async fn alert(&self, id: &str, customer: &str, key: &str) -> Result<(), String> {
        self.exec(&alert_sql(
            id,
            customer,
            key,
            "SELL",
            "ABOVE",
            "34.00000000",
            "1",
        ))
        .await
    }

    async fn event(&self, line: &str, episode: i32) -> Result<(), String> {
        self.exec(&event_sql(line, episode)).await
    }
}

fn alert_sql(
    id: &str,
    customer: &str,
    key: &str,
    side: &str,
    condition: &str,
    target: &str,
    unit: &str,
) -> String {
    format!(
        "INSERT INTO rate_alerts (id, organization_id, customer_id, branch_id, currency_code, shop_side, unit_size, \
         condition, target, cooldown_seconds, expires_at, idempotency_key) VALUES \
         ('{id}', '{ORG}', '{customer}', 'MAIN', 'USD', '{side}', {unit}, '{condition}', {target}, 86400, \
          now() + interval '30 days', '{key}')"
    )
}

fn event_sql(line: &str, episode: i32) -> String {
    format!(
        "INSERT INTO rate_alert_events (id, organization_id, alert_id, published_rate_line_id, observed_rate, \
         trigger_at, episode_no) VALUES (gen_random_uuid(), '{ORG}', '{ALERT}', '{line}', 34.10000000, now(), \
         {episode})"
    )
}

fn settings_sql(cooldown: &str, max: &str, lifetime: &str, policy: &str) -> String {
    format!(
        "INSERT INTO rate_alert_settings (organization_id, default_cooldown_seconds, max_active_per_customer, \
         default_lifetime_days, observe_policy, updated_by, updated_at) VALUES \
         ('{ORG}', {cooldown}, {max}, {lifetime}, '{policy}', 'c1000000-0000-0000-0000-000000000001', now())"
    )
}

#[tokio::test]
async fn the_migration_is_one_file_above_mains_newest_when_cut() {
    let version = rate_alerts_version();
    assert!(
        version > NEWEST_WHEN_CUT,
        "<version>_rate_alerts.sql sorts after {NEWEST_WHEN_CUT} (migration-order gate): {version}"
    );
}

#[tokio::test]
async fn a_fresh_database_has_the_three_tables_and_no_settings_row() {
    let db = Db::fresh().await;
    for table in ["rate_alert_settings", "rate_alerts", "rate_alert_events"] {
        assert_eq!(
            db.count(&format!(
                "SELECT count(*) FROM information_schema.tables \
                 WHERE table_schema = current_schema() AND table_name = '{table}'"
            ))
            .await,
            1,
            "{table} exists"
        );
    }
    assert_eq!(
        db.count("SELECT count(*) FROM rate_alert_settings").await,
        0,
        "§8 M1 item 1: no settings row is seeded (updated_by must be a person)"
    );
}

#[tokio::test]
async fn a_new_alert_starts_unknown_enabled_episode_zero_version_one() {
    let db = Db::fresh().await;
    db.world().await;
    db.alert(ALERT, "CX-1", "k1")
        .await
        .expect("a valid alert is accepted");
    let row = db
        .text(&format!(
            "SELECT relation || '|' || enabled || '|' || episode_no || '|' || version || '|' || \
             coalesce(last_publication_version::text, 'null') || '|' || coalesce(disabled_reason, 'null') || '|' || \
             coalesce(deleted_at::text, 'null') || '|' || coalesce(last_triggered_at::text, 'null') \
             FROM rate_alerts WHERE id = '{ALERT}'"
        ))
        .await;
    assert_eq!(row, "UNKNOWN|true|0|1|null|null|null|null");
}

#[tokio::test]
async fn shop_side_condition_relation_and_disabled_reason_refuse_other_values() {
    let db = Db::fresh().await;
    db.world().await;
    db.ok(
        &alert_sql(ALERT, "CX-1", "k1", "BUY", "BELOW", "33.00000000", "1"),
        "BUY / BELOW",
    )
    .await;
    db.refused(
        &alert_sql(
            "a1000000-0000-0000-0000-000000000002",
            "CX-1",
            "k2",
            "CUSTOMER_BUY",
            "ABOVE",
            "34",
            "1",
        ),
        CHECK_VIOLATION,
        "a customer-side word stored as shop_side",
    )
    .await;
    db.refused(
        &alert_sql(
            "a1000000-0000-0000-0000-000000000003",
            "CX-1",
            "k3",
            "SELL",
            "EQUAL",
            "34",
            "1",
        ),
        CHECK_VIOLATION,
        "condition EQUAL",
    )
    .await;
    db.refused(
        &format!("UPDATE rate_alerts SET relation = 'CROSSED' WHERE id = '{ALERT}'"),
        CHECK_VIOLATION,
        "relation CROSSED",
    )
    .await;
    db.ok(
        &format!("UPDATE rate_alerts SET relation = 'SATISFIED' WHERE id = '{ALERT}'"),
        "relation SATISFIED",
    )
    .await;
    db.ok(
        &format!("UPDATE rate_alerts SET relation = 'NOT_SATISFIED' WHERE id = '{ALERT}'"),
        "relation NOT_SATISFIED",
    )
    .await;
    db.ok(
        &format!(
            "UPDATE rate_alerts SET enabled = false, disabled_reason = 'UNIT_CHANGED' WHERE id = '{ALERT}'"
        ),
        "disabled_reason UNIT_CHANGED",
    )
    .await;
    db.refused(
        &format!("UPDATE rate_alerts SET disabled_reason = 'PAUSED' WHERE id = '{ALERT}'"),
        CHECK_VIOLATION,
        "disabled_reason PAUSED",
    )
    .await;
}

#[tokio::test]
async fn a_target_or_unit_size_of_zero_and_a_negative_cooldown_are_refused() {
    let db = Db::fresh().await;
    db.world().await;
    db.ok(
        &alert_sql(ALERT, "CX-1", "k1", "SELL", "ABOVE", "0.00000001", "1"),
        "the smallest positive target",
    )
    .await;
    db.refused(
        &alert_sql(
            "a1000000-0000-0000-0000-000000000002",
            "CX-1",
            "k2",
            "SELL",
            "ABOVE",
            "0",
            "1",
        ),
        CHECK_VIOLATION,
        "target 0",
    )
    .await;
    db.refused(
        &alert_sql(
            "a1000000-0000-0000-0000-000000000003",
            "CX-1",
            "k3",
            "SELL",
            "ABOVE",
            "34",
            "0",
        ),
        CHECK_VIOLATION,
        "unit_size 0",
    )
    .await;
    db.refused(
        &format!("UPDATE rate_alerts SET cooldown_seconds = -1 WHERE id = '{ALERT}'"),
        CHECK_VIOLATION,
        "cooldown_seconds -1",
    )
    .await;
}

#[tokio::test]
async fn the_target_keeps_eight_places_per_unit_size() {
    let db = Db::fresh().await;
    db.world().await;
    db.ok(
        &alert_sql(ALERT, "CX-1", "k1", "SELL", "ABOVE", "23.12345678", "100"),
        "a JPY-style target per 100",
    )
    .await;
    assert_eq!(
        db.text(&format!(
            "SELECT target::text || '|' || unit_size::text FROM rate_alerts WHERE id = '{ALERT}'"
        ))
        .await,
        "23.12345678|100.0000",
        "§4.1: target numeric(20,8), unit_size numeric(20,4); nothing divides by unit_size (R2)"
    );
}

#[tokio::test]
async fn one_idempotency_key_per_customer() {
    let db = Db::fresh().await;
    db.world().await;
    db.alert(ALERT, "CX-1", "k1")
        .await
        .expect("the first alert with key k1");
    db.ok(
        &alert_sql(
            "a1000000-0000-0000-0000-000000000002",
            "CX-2",
            "k1",
            "SELL",
            "ABOVE",
            "34",
            "1",
        ),
        "another customer's alert with the same key",
    )
    .await;
    db.refused(
        &alert_sql(
            "a1000000-0000-0000-0000-000000000003",
            "CX-1",
            "k1",
            "BUY",
            "BELOW",
            "30",
            "1",
        ),
        UNIQUE_VIOLATION,
        "a second alert of CX-1 with key k1",
    )
    .await;
}

#[tokio::test]
async fn an_alert_needs_a_real_customer_branch_of_its_organisation_and_currency() {
    let db = Db::fresh().await;
    db.world().await;
    db.refused(
        &alert_sql(ALERT, "CX-NOBODY", "k1", "SELL", "ABOVE", "34", "1"),
        FOREIGN_KEY_VIOLATION,
        "an unknown customer",
    )
    .await;
    db.ok(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ('00000000-0000-0000-0000-000000000002', 'OTHER', 'Asia/Bangkok', now())",
        "a second organisation",
    )
    .await;
    db.refused(
        &alert_sql(ALERT, "CX-1", "k1", "SELL", "ABOVE", "34", "1").replace(
            &format!("'{ORG}', 'CX-1', 'MAIN'"),
            "'00000000-0000-0000-0000-000000000002', 'CX-1', 'MAIN'",
        ),
        FOREIGN_KEY_VIOLATION,
        "branch MAIN under an organisation it does not belong to",
    )
    .await;
    db.refused(
        &alert_sql(ALERT, "CX-1", "k1", "SELL", "ABOVE", "34", "1")
            .replace("'MAIN', 'USD'", "'MAIN', 'ZZZ'"),
        FOREIGN_KEY_VIOLATION,
        "an unknown currency",
    )
    .await;
}

#[tokio::test]
async fn at_most_one_event_per_episode_and_per_published_line() {
    let db = Db::fresh().await;
    db.world().await;
    db.alert(ALERT, "CX-1", "k1").await.expect("the alert");
    db.event(LINE, 1).await.expect("episode 1 on line 1");
    db.refused(
        &event_sql(LINE2, 1),
        UNIQUE_VIOLATION,
        "a second event for episode 1 (R6)",
    )
    .await;
    db.refused(
        &event_sql(LINE, 2),
        UNIQUE_VIOLATION,
        "a second event for the same published line (R13)",
    )
    .await;
    db.event(LINE2, 2).await.expect("episode 2 on line 2");
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM rate_alert_events WHERE alert_id = '{ALERT}'"
        ))
        .await,
        2
    );
}

#[tokio::test]
async fn an_event_needs_a_positive_episode_a_real_alert_and_a_real_line() {
    let db = Db::fresh().await;
    db.world().await;
    db.alert(ALERT, "CX-1", "k1").await.expect("the alert");
    db.refused(&event_sql(LINE, 0), CHECK_VIOLATION, "episode_no 0")
        .await;
    db.refused(
        &event_sql("b1000000-0000-0000-0000-0000000000ff", 1),
        FOREIGN_KEY_VIOLATION,
        "an unknown published line",
    )
    .await;
    db.refused(
        &event_sql(LINE, 1).replace(ALERT, "a1000000-0000-0000-0000-0000000000ff"),
        FOREIGN_KEY_VIOLATION,
        "an unknown alert",
    )
    .await;
}

#[tokio::test]
async fn settings_hold_the_owners_values_and_refuse_bad_ones() {
    let db = Db::fresh().await;
    db.world().await;
    db.refused(
        &settings_sql("86400", "10", "30", "MANUAL_ONLY"),
        CHECK_VIOLATION,
        "observe_policy MANUAL_ONLY",
    )
    .await;
    db.refused(
        &settings_sql("86400", "0", "30", "ALL_FRESH"),
        CHECK_VIOLATION,
        "max_active_per_customer 0",
    )
    .await;
    db.refused(
        &settings_sql("-1", "10", "30", "ALL_FRESH"),
        CHECK_VIOLATION,
        "default_cooldown_seconds -1",
    )
    .await;
    db.refused(
        &settings_sql("86400", "10", "0", "ALL_FRESH"),
        CHECK_VIOLATION,
        "default_lifetime_days 0",
    )
    .await;
    db.refused(
        &settings_sql("86400", "10", "30", "ALL_FRESH").replace(
            "c1000000-0000-0000-0000-000000000001",
            "c1000000-0000-0000-0000-0000000000ff",
        ),
        FOREIGN_KEY_VIOLATION,
        "updated_by naming nobody",
    )
    .await;
    db.ok(
        &settings_sql("86400", "10", "30", "ALL_FRESH"),
        "the owner's 2026-10-10 values (§0)",
    )
    .await;
    db.refused(
        &settings_sql("3600", "5", "NULL", "ALL"),
        UNIQUE_VIOLATION,
        "a second settings row for the organisation",
    )
    .await;
    db.ok(
        "UPDATE rate_alert_settings SET default_lifetime_days = NULL, observe_policy = 'AUTO_FRESH'",
        "no expiry (NULL lifetime) and AUTO_FRESH",
    )
    .await;
}
