//! APP-1 (#558): the customer app's schema and M1. Written before the implementation by the ticket
//! author (claude/5edf7bb5). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/customer-app.md` §4.1 (tables), §8 (M1), §9.1 row APP-1, owner C4
//! (a planner draft lives 7 days, stored per organisation in `customer_app_settings`).
//!
//! The test lives in `xc-kernel` only because that crate already applies `api/migrations/` in its own
//! tests (`branch_tx_test.rs`) and builds fast; it tests no kernel code.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing database whose name
//! ends in `_test`. They fail without it; they do not skip. Each test works in a schema of its own.

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

const MUST_FINISH: Duration = Duration::from_secs(30);
/// OR-1's install organisation (`20261011100000_organizations.sql`).
const XC: &str = "00000000-0000-0000-0000-000000000001";
/// Owner C4: 7 days.
const SEVEN_DAYS: i32 = 604_800;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("custapp");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let migrator = Migrator::new(dir.as_path())
            .await
            .expect("read api/migrations");
        migrator
            .run(&pool)
            .await
            .unwrap_or_else(|e| panic!("apply api/migrations in version order: {e}"));
        Self { pool, schema }
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    async fn exec(&self, sql: &str) -> Result<(), sqlx::Error> {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    async fn ok(&self, what: &str, sql: &str) {
        if let Err(e) = self.exec(sql).await {
            panic!("{what} must be accepted, got: {e}\nSQL: {sql}");
        }
    }

    /// The statement must fail with a constraint violation (23xxx), not with a missing table or column.
    async fn refused(&self, what: &str, sql: &str) {
        match self.exec(sql).await {
            Ok(()) => {
                panic!("{what} must be refused by the schema, but it was accepted.\nSQL: {sql}")
            }
            Err(sqlx::Error::Database(db)) => {
                let code = db.code().map(|c| c.into_owned()).unwrap_or_default();
                assert!(
                    code.starts_with("23"),
                    "{what} must be refused by a constraint (SQLSTATE 23xxx), got {code}: {}\nSQL: {sql}",
                    db.message()
                );
            }
            Err(e) => panic!("{what}: unexpected error {e}"),
        }
    }

    /// A customer of XC with a user, for owner columns. Returns `(customer id, user id)`.
    async fn customer_with_user(&self) -> (String, String) {
        let user: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, 'App Customer', 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(XC)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        let customer = unique("C");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
             VALUES ($1, $2::uuid, $1, 'App Customer', 'LIVE', now(), $3::uuid)",
        )
        .bind(&customer)
        .bind(XC)
        .bind(&user)
        .execute(&self.pool)
        .await
        .expect("insert a customer");
        (customer, user)
    }
}

fn draft(owner_cols: &str, owner_vals: &str, input_cols: &str, input_vals: &str) -> String {
    format!(
        "INSERT INTO planner_drafts (id, organization_id, {owner_cols}, currency_code, customer_side, {input_cols}, expires_at) \
         VALUES (gen_random_uuid(), '{XC}', {owner_vals}, 'USD', 'CUSTOMER_BUY', {input_vals}, now() + interval '7 days')"
    )
}

#[tokio::test]
async fn every_migration_applies_and_creates_the_six_tables() {
    let db = TestDb::start().await;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = current_schema() AND table_name = ANY($1) ORDER BY table_name",
    )
    .bind(vec![
        "assistant_conversations",
        "assistant_messages",
        "customer_app_settings",
        "notification_deliveries",
        "notifications",
        "planner_drafts",
    ])
    .fetch_all(&db.pool)
    .await
    .expect("read information_schema");
    assert_eq!(
        tables,
        vec![
            "assistant_conversations",
            "assistant_messages",
            "customer_app_settings",
            "notification_deliveries",
            "notifications",
            "planner_drafts",
        ],
        "M1 creates exactly these six tables (design §4.1)"
    );
    db.finish().await;
}

#[tokio::test]
async fn each_organisation_gets_one_settings_row_of_seven_days() {
    let db = TestDb::start().await;
    let orgs: i64 = sqlx::query_scalar("SELECT count(*) FROM organizations")
        .fetch_one(&db.pool)
        .await
        .expect("count organizations");
    let rows: Vec<(String, Option<i32>)> = sqlx::query_as(
        "SELECT organization_id::text, planner_draft_ttl_seconds FROM customer_app_settings ORDER BY 1",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("read customer_app_settings: {e}"));
    assert_eq!(
        rows.len() as i64,
        orgs,
        "one settings row per existing organisation"
    );
    assert!(
        rows.iter().any(|(org, _)| org == XC),
        "the install organisation XC has its row"
    );
    for (org, ttl) in &rows {
        assert_eq!(
            *ttl,
            Some(SEVEN_DAYS),
            "organisation {org} is seeded with 7 days (owner C4)"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn a_draft_lifetime_of_zero_is_refused_and_null_means_not_set() {
    let db = TestDb::start().await;
    db.refused(
        "planner_draft_ttl_seconds = 0",
        &format!("UPDATE customer_app_settings SET planner_draft_ttl_seconds = 0 WHERE organization_id = '{XC}'"),
    )
    .await;
    db.refused(
        "a second settings row for one organisation",
        &format!(
            "INSERT INTO customer_app_settings (organization_id, planner_draft_ttl_seconds, updated_by, updated_at) \
             VALUES ('{XC}', 60, 'test', now())"
        ),
    )
    .await;
    db.ok(
        "planner_draft_ttl_seconds = NULL (not set)",
        &format!("UPDATE customer_app_settings SET planner_draft_ttl_seconds = NULL WHERE organization_id = '{XC}'"),
    )
    .await;
    db.finish().await;
}

#[tokio::test]
async fn a_draft_has_exactly_one_owner() {
    let db = TestDb::start().await;
    let (customer, _) = db.customer_with_user().await;
    db.refused(
        "a draft with both a customer and a session",
        &draft(
            "customer_id, session_hash",
            &format!("'{customer}', 'h1'"),
            "quantity",
            "500",
        ),
    )
    .await;
    db.refused(
        "a draft with neither a customer nor a session",
        &draft("customer_id, session_hash", "NULL, NULL", "quantity", "500"),
    )
    .await;
    db.ok(
        "a customer-owned draft",
        &draft("customer_id", &format!("'{customer}'"), "quantity", "500"),
    )
    .await;
    db.ok(
        "a session-owned draft",
        &draft("session_hash", "'h2'", "budget_thb", "10000"),
    )
    .await;
    db.finish().await;
}

#[tokio::test]
async fn a_draft_has_exactly_one_input_and_a_valid_side() {
    let db = TestDb::start().await;
    db.refused(
        "a draft with both quantity and budget",
        &draft("session_hash", "'h1'", "quantity, budget_thb", "500, 10000"),
    )
    .await;
    db.refused(
        "a draft with neither quantity nor budget",
        &draft("session_hash", "'h1'", "quantity, budget_thb", "NULL, NULL"),
    )
    .await;
    db.refused(
        "a draft with quantity 0",
        &draft("session_hash", "'h1'", "quantity", "0"),
    )
    .await;
    db.refused(
        "a shop-side value in customer_side",
        &format!(
            "INSERT INTO planner_drafts (id, organization_id, session_hash, currency_code, customer_side, quantity, expires_at) \
             VALUES (gen_random_uuid(), '{XC}', 'h1', 'USD', 'BUY', 500, now())"
        ),
    )
    .await;
    db.refused(
        "a currency that is not in currencies",
        &format!(
            "INSERT INTO planner_drafts (id, organization_id, session_hash, currency_code, customer_side, quantity, expires_at) \
             VALUES (gen_random_uuid(), '{XC}', 'h1', 'ZZZ', 'CUSTOMER_SELL', 500, now())"
        ),
    )
    .await;
    db.finish().await;
}

#[tokio::test]
async fn a_notification_is_unique_per_recipient_event_and_template() {
    let db = TestDb::start().await;
    let (_, user) = db.customer_with_user().await;
    let insert = |event: &str, template: &str, locale: &str| {
        format!(
            "INSERT INTO notifications (id, organization_id, recipient_user_id, event_id, template_key, template_version, locale) \
             VALUES (gen_random_uuid(), '{XC}', '{user}', '{event}', '{template}', 1, '{locale}')"
        )
    };
    db.ok(
        "a first notification",
        &insert("queue.called:T1:1", "queue_called", "zh"),
    )
    .await;
    db.refused(
        "a second notification with the same (recipient, event, template)",
        &insert("queue.called:T1:1", "queue_called", "th"),
    )
    .await;
    db.ok(
        "the same event with another template",
        &insert("queue.called:T1:1", "other_template", "th"),
    )
    .await;
    db.ok(
        "a recall (another event id)",
        &insert("queue.called:T1:2", "queue_called", "th"),
    )
    .await;
    db.refused(
        "a locale outside th/en/zh",
        &insert("e:9", "queue_called", "fr"),
    )
    .await;
    let seqs: Vec<i64> = sqlx::query_scalar("SELECT seq FROM notifications ORDER BY seq")
        .fetch_all(&db.pool)
        .await
        .expect("read notifications.seq");
    assert_eq!(seqs.len(), 3);
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "seq is generated and increasing: {seqs:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_notification_has_one_delivery_per_channel() {
    let db = TestDb::start().await;
    let (_, user) = db.customer_with_user().await;
    let id: String = sqlx::query_scalar(
        "INSERT INTO notifications (id, organization_id, recipient_user_id, event_id, template_key, template_version, locale) \
         VALUES (gen_random_uuid(), $1::uuid, $2::uuid, 'e:1', 'queue_called', 1, 'th') RETURNING id::text",
    )
    .bind(XC)
    .bind(&user)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("insert a notification: {e}"));
    let delivery = |channel: &str, status: &str| {
        format!(
            "INSERT INTO notification_deliveries (id, organization_id, notification_id, channel, status) \
             VALUES (gen_random_uuid(), '{XC}', '{id}', '{channel}', '{status}')"
        )
    };
    db.ok("an IN_APP delivery", &delivery("IN_APP", "SENT"))
        .await;
    db.refused("a second IN_APP delivery", &delivery("IN_APP", "PENDING"))
        .await;
    db.refused("an unknown channel", &delivery("FAX", "PENDING"))
        .await;
    db.refused("an unknown status", &delivery("EMAIL", "DONE"))
        .await;
    db.ok("an EMAIL delivery beside it", &delivery("EMAIL", "SKIPPED"))
        .await;
    let attempt: i32 = sqlx::query_scalar(
        "SELECT attempt_no FROM notification_deliveries WHERE channel = 'IN_APP'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read attempt_no");
    assert_eq!(attempt, 0, "attempt_no defaults to 0");
    db.finish().await;
}

#[tokio::test]
async fn a_conversation_has_one_owner_and_a_message_key_is_used_once() {
    let db = TestDb::start().await;
    db.refused(
        "a conversation with neither owner",
        &format!(
            "INSERT INTO assistant_conversations (id, organization_id, locale) VALUES (gen_random_uuid(), '{XC}', 'th')"
        ),
    )
    .await;
    let conv: String = sqlx::query_scalar(
        "INSERT INTO assistant_conversations (id, organization_id, session_hash, locale) \
         VALUES (gen_random_uuid(), $1::uuid, 'h1', 'en') RETURNING id::text",
    )
    .bind(XC)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("insert a session conversation: {e}"));
    let message = |role: &str, key: &str| {
        format!(
            "INSERT INTO assistant_messages (id, organization_id, conversation_id, role, redacted_text, idempotency_key) \
             VALUES (gen_random_uuid(), '{XC}', '{conv}', '{role}', 'USD 500', {key})"
        )
    };
    db.ok("a first message", &message("CUSTOMER", "'k1'")).await;
    db.refused(
        "a second message with the same key",
        &message("CUSTOMER", "'k1'"),
    )
    .await;
    db.ok(
        "an assistant answer without a key",
        &message("ASSISTANT", "NULL"),
    )
    .await;
    db.refused("an unknown role", &message("AGENT", "'k2'"))
        .await;
    db.finish().await;
}
