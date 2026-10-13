//! Ticket #1584 (SM-2) — sharp price move must raise a notice on publish.
//! Design: `docs/design/blueprint/rates.md` §12.8 (rules R-12f), §12.10 (SM-2 row), and issue #1584 §9.
//!
//! Written before implementation by the up-front test-writer gemini/pool-3, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! **Behaviour table coverage (§9):**
//! 1. 3% move with a 2% threshold: every org-wide EXECUTIVE and ADMIN of the branch's organisation receives
//!    one notification with template `rate_sharp_move` and nobody else.
//! 2. A branch-scoped ADMIN: not told (`branch_id IS NULL` is required in `user_roles`).
//! 3. Threshold NULL: no notice (`sharp_move_alert_pct IS NULL`).
//! 4. Move exactly at the threshold: no notice (`move_pct > threshold` strictly greater).
//! 5. The notice fails to write: the publication still commits and other notices are delivered
//!    (notices written under SAVEPOINT, lane controller ruling #1584 comment-6137651681).
//! 6. The publish rolls back: no notice written (notices inside publish transaction roll back with it).
//! 7. A retry of the same publish: no second notice (`event_id` idempotency `"rate-sharp-move:<pub_id>:<currency>:<side>"`).
//! 8. Rates with 4 or more integer digits (USD 1000 -> 1040, +4%, threshold 2%): publication commits and org-wide EXECUTIVE receives notice.
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate, Side};
use xc_notify::application::notification_service::NotificationService;
use xc_notify::application::template_registry::TemplateRegistry;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::application::publication::sharp_move_notices::SharpMoveNotices;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::draft::rate_draft::{DraftId, DraftLine, LineSide};
use xc_rate::domain::publication::sharp_move::Move;
use xchange::presentation::http::v3::commands::make_approval_service;

const MUST_FINISH: Duration = Duration::from_secs(30);
const MAIN: &str = "MAIN";
const ORG_ID: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ── Test Database Harness ───────────────────────────────────────────────────

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The SM-2 rate sharp move notice tests need a real PostgreSQL and do not skip. Point it \
                 at an existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("ratesharpnotices");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");
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
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn owner_actor() -> Actor {
    Actor::new(
        "owner-test-user",
        ORG_ID,
        vec![
            ScopedRole::new(Role::Executive, None, DateTime::<Utc>::UNIX_EPOCH, None),
            ScopedRole::new(Role::Admin, None, DateTime::<Utc>::UNIX_EPOCH, None),
        ],
        None,
    )
}

async fn create_user_with_org_and_role(
    pool: &PgPool,
    org_id: &str,
    name: &str,
    role_code: &str,
    branch_id: Option<&str>,
    valid_to: Option<DateTime<Utc>>,
) -> Uuid {
    let user_id = Uuid::new_v4();
    let org_uuid = Uuid::parse_str(org_id).unwrap();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2, $3, 'ACTIVE', 'test')",
    )
    .bind(user_id)
    .bind(org_uuid)
    .bind(name)
    .execute(pool)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO user_roles (id, user_id, role_code, organization_id, branch_id, valid_from, valid_to) \
         VALUES (gen_random_uuid(), $1, $2, $3, $4, now() - interval '1 day', $5)",
    )
    .bind(user_id)
    .bind(role_code)
    .bind(org_uuid)
    .bind(branch_id)
    .bind(valid_to)
    .execute(pool)
    .await
    .expect("insert user_roles");

    user_id
}

async fn create_user_with_role(
    pool: &PgPool,
    name: &str,
    role_code: &str,
    branch_id: Option<&str>,
    valid_to: Option<DateTime<Utc>>,
) -> Uuid {
    create_user_with_org_and_role(pool, ORG_ID, name, role_code, branch_id, valid_to).await
}

async fn seed_policy(
    pool: &PgPool,
    branch: &str,
    version: i32,
    sharp_move_alert_pct: Option<&str>,
) {
    let guards = json!({
        "USD": {
            "buy": { "min_pct": "-50.0000", "max_pct": "50.0000" },
            "sell": { "min_pct": "-50.0000", "max_pct": "50.0000" }
        },
        "EUR": {
            "buy": { "min_pct": "-50.0000", "max_pct": "50.0000" },
            "sell": { "min_pct": "-50.0000", "max_pct": "50.0000" }
        }
    });
    let alert_pct_dec = sharp_move_alert_pct.map(|s| Decimal::from_str(s).unwrap());
    sqlx::query(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
            valid_from, valid_to, approved_by, reason, sharp_move_alert_pct) \
         VALUES (gen_random_uuid(), $1, $2, 0, 0, $3, now() - interval '1 day', NULL, 'owner', 'seed', $4)",
    )
    .bind(branch)
    .bind(version)
    .bind(guards)
    .bind(alert_pct_dec)
    .execute(pool)
    .await
    .expect("seed rate policy version");
}

async fn create_and_submit_approved_draft(
    pool: &PgPool,
    branch: &str,
    lines: &[(&str, &str, &str)],
) -> (DraftId, Option<i32>) {
    let b = BranchId::new(branch);
    let owner_user = owner_actor();
    let drafts = RateDraftService::new(pool.clone(), make_approval_service(pool.clone()));
    let draft_lines: Vec<DraftLine> = lines
        .iter()
        .map(|(code, buy, sell)| {
            DraftLine::new(
                CurrencyCode::parse(code).unwrap_or_else(|_| panic!("{code} is a currency")),
                LineSide::Manual(Rate::parse(buy).unwrap_or_else(|e| panic!("{buy}: {e:?}"))),
                LineSide::Manual(Rate::parse(sell).unwrap_or_else(|e| panic!("{sell}: {e:?}"))),
                Some("published by SM-2 test".to_owned()),
            )
        })
        .collect();

    let mut tx = pool.begin().await.expect("begin create draft");
    let draft = drafts
        .create(&mut tx, &b, &owner_user, draft_lines)
        .await
        .unwrap_or_else(|e| panic!("create the draft: {e:?}"));
    tx.commit().await.expect("commit the draft");

    let unique_key = format!("sm2_submit_{}", Uuid::new_v4());
    let mut tx = pool.begin().await.expect("begin submit draft");
    drafts
        .submit(&mut tx, &b, &owner_user, &draft.id, &unique_key)
        .await
        .unwrap_or_else(|e| panic!("submit the draft: {e:?}"));
    tx.commit().await.expect("commit the submit");

    let expected: Option<i32> = sqlx::query_scalar(
        "SELECT p.version FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = $1",
    )
    .bind(branch)
    .fetch_optional(pool)
    .await
    .expect("read current version");

    (draft.id, expected)
}

async fn publish_rates(pool: &PgPool, branch: &str, lines: &[(&str, &str, &str)]) -> String {
    let (draft_id, expected) = create_and_submit_approved_draft(pool, branch, lines).await;
    let b = BranchId::new(branch);
    let owner_user = owner_actor();
    let publisher = PublishService::new(pool.clone());

    let mut tx = pool.begin().await.expect("begin publish");
    let published = publisher
        .publish(&mut tx, &b, &owner_user, &draft_id, expected)
        .await
        .unwrap_or_else(|e| panic!("publish the draft: {}", e.code()));
    tx.commit().await.expect("commit the publish");

    published.id.to_string()
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct NotificationRow {
    #[allow(dead_code)]
    id: Uuid,
    #[allow(dead_code)]
    organization_id: Uuid,
    #[allow(dead_code)]
    recipient_user_id: Uuid,
    event_id: String,
    template_key: String,
    #[allow(dead_code)]
    template_version: i32,
    object_type: Option<String>,
    object_id: Option<String>,
    params: Value,
}

async fn sharp_move_notifications_for_pub(
    pool: &PgPool,
    publication_id: &str,
) -> Vec<NotificationRow> {
    sqlx::query_as::<_, NotificationRow>(
        "SELECT id, organization_id, recipient_user_id, event_id, template_key, \
                template_version, object_type, object_id, params \
         FROM notifications \
         WHERE template_key = 'rate_sharp_move' AND object_id = $1 \
         ORDER BY recipient_user_id",
    )
    .bind(publication_id)
    .fetch_all(pool)
    .await
    .expect("query sharp move notifications")
}

async fn sharp_move_notifications_for_user(
    pool: &PgPool,
    user_id: Uuid,
    publication_id: &str,
) -> Vec<NotificationRow> {
    sqlx::query_as::<_, NotificationRow>(
        "SELECT id, organization_id, recipient_user_id, event_id, template_key, \
                template_version, object_type, object_id, params \
         FROM notifications \
         WHERE recipient_user_id = $1 AND template_key = 'rate_sharp_move' AND object_id = $2 \
         ORDER BY id",
    )
    .bind(user_id)
    .bind(publication_id)
    .fetch_all(pool)
    .await
    .expect("query sharp move notifications for user")
}

fn assert_sharp_move_params(
    n: &NotificationRow,
    expected_pub_id: &str,
    expected_branch: &str,
    expected_currency: &str,
    expected_side: &str,
) {
    assert_eq!(
        n.object_type.as_deref(),
        Some("rate_publication"),
        "object_type must be rate_publication"
    );
    assert_eq!(
        n.object_id.as_deref(),
        Some(expected_pub_id),
        "object_id must match publication id"
    );

    let params = n.params.as_object().expect("params must be a JSON object");
    let mut keys: Vec<&str> = params.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec!["branch", "currency", "side"],
        "params must equal exactly {{branch, currency, side}}, got keys: {:?}",
        keys
    );

    assert_eq!(
        params.get("branch").and_then(Value::as_str),
        Some(expected_branch),
        "branch param must match"
    );
    assert_eq!(
        params.get("currency").and_then(Value::as_str),
        Some(expected_currency),
        "currency param must match"
    );
    assert_eq!(
        params.get("side").and_then(Value::as_str),
        Some(expected_side),
        "side param must match"
    );

    // Assert that no numeric key is present
    let numeric_keys = [
        "old_rate",
        "old",
        "new_rate",
        "new",
        "move_pct",
        "move",
        "threshold",
        "alert_threshold",
        "sharp_move_alert_pct",
        "version",
        "publication_version",
    ];
    for nk in numeric_keys {
        assert!(
            !params.contains_key(nk),
            "numeric key '{nk}' must not be present in params"
        );
    }
    for (k, v) in params {
        assert!(
            !v.is_number(),
            "no numeric key or value allowed in params, found {k}: {v:?}"
        );
    }
}

// ── Specification Tests ─────────────────────────────────────────────────────

/// Row 1: A 3% move with a 2% threshold notifies every org-wide EXECUTIVE and ADMIN
/// of the branch's organisation once with template `rate_sharp_move`, and nobody else.
/// Also tests BUY and SELL side moves, second organisation exclusion, and all §12.8 notification params.
#[tokio::test]
async fn sharp_move_3_pct_with_2_pct_threshold_notifies_org_executive_and_admin() {
    let db = TestDb::start().await;

    // Recipients in the branch's organisation:
    // - org-wide EXECUTIVE (should receive notice)
    // - org-wide ADMIN (should receive notice)
    // - org-wide TELLER (should NOT receive notice: not EXECUTIVE/ADMIN)
    // - org-wide ADMIN with expired valid_to (should NOT receive notice)
    let exec_id = create_user_with_role(&db.pool, "Executive Org", "EXECUTIVE", None, None).await;
    let admin_id = create_user_with_role(&db.pool, "Admin Org", "ADMIN", None, None).await;
    let teller_id = create_user_with_role(&db.pool, "Teller Org", "TELLER", None, None).await;
    let expired_admin_id = create_user_with_role(
        &db.pool,
        "Expired Admin",
        "ADMIN",
        None,
        Some(Utc::now() - chrono::Duration::hours(1)),
    )
    .await;

    // Second organisation: org-wide ADMIN in another organisation (must NOT receive notice)
    let org2_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ($1, 'ORG2', 'Asia/Bangkok', now())",
    )
    .bind(org2_id)
    .execute(&db.pool)
    .await
    .expect("insert org2");

    let other_org_admin = create_user_with_org_and_role(
        &db.pool,
        &org2_id.to_string(),
        "Other Org Admin",
        "ADMIN",
        None,
        None,
    )
    .await;

    seed_policy(&db.pool, MAIN, 1, Some("2.0000")).await;

    // Baseline publication 1: USD buy 30.0000, sell 31.0000.
    // First publication has no previous publication to compare with -> no sharp move notice.
    let pub1_id = publish_rates(&db.pool, MAIN, &[("USD", "30.0000", "31.0000")]).await;
    let pub1_notices = sharp_move_notifications_for_pub(&db.pool, &pub1_id).await;
    assert_eq!(
        pub1_notices.len(),
        0,
        "first publication has no previous rates to compare with"
    );

    // Publication 2: Both USD BUY and SELL move sharply:
    // USD buy moves from 30.0000 to 31.0000 (3.33% move > 2% threshold).
    // USD sell moves from 31.0000 to 32.0000 (3.23% move > 2% threshold).
    // Buy <= sell (31.0000 <= 32.0000) is valid.
    let pub2_id = publish_rates(&db.pool, MAIN, &[("USD", "31.0000", "32.0000")]).await;

    let pub2_notices = sharp_move_notifications_for_pub(&db.pool, &pub2_id).await;
    assert_eq!(
        pub2_notices.len(),
        4,
        "expected exactly 4 notifications (BUY and SELL for EXECUTIVE and ADMIN), found {}",
        pub2_notices.len()
    );

    let find_notice = |notices: &[NotificationRow], side: &str| -> NotificationRow {
        notices
            .iter()
            .find(|n| {
                n.params.get("side").and_then(Value::as_str) == Some(side)
                    || n.event_id.ends_with(&format!(":{side}"))
            })
            .cloned()
            .unwrap_or_else(|| panic!("notice for side {side} not found in {notices:?}"))
    };

    // Assert executive received BUY and SELL sharp move notifications
    let exec_notices = sharp_move_notifications_for_user(&db.pool, exec_id, &pub2_id).await;
    assert_eq!(
        exec_notices.len(),
        2,
        "org-wide EXECUTIVE must receive 2 notifications (BUY and SELL)"
    );
    let exec_buy = find_notice(&exec_notices, "BUY");
    assert_eq!(exec_buy.template_key, "rate_sharp_move");
    assert_eq!(
        exec_buy.event_id,
        format!("rate-sharp-move:{pub2_id}:USD:BUY")
    );
    assert_sharp_move_params(&exec_buy, &pub2_id, MAIN, "USD", "BUY");

    let exec_sell = find_notice(&exec_notices, "SELL");
    assert_eq!(exec_sell.template_key, "rate_sharp_move");
    assert_eq!(
        exec_sell.event_id,
        format!("rate-sharp-move:{pub2_id}:USD:SELL")
    );
    assert_sharp_move_params(&exec_sell, &pub2_id, MAIN, "USD", "SELL");

    // Assert admin received BUY and SELL sharp move notifications
    let admin_notices = sharp_move_notifications_for_user(&db.pool, admin_id, &pub2_id).await;
    assert_eq!(
        admin_notices.len(),
        2,
        "org-wide ADMIN must receive 2 notifications (BUY and SELL)"
    );
    let admin_buy = find_notice(&admin_notices, "BUY");
    assert_eq!(admin_buy.template_key, "rate_sharp_move");
    assert_eq!(
        admin_buy.event_id,
        format!("rate-sharp-move:{pub2_id}:USD:BUY")
    );
    assert_sharp_move_params(&admin_buy, &pub2_id, MAIN, "USD", "BUY");

    let admin_sell = find_notice(&admin_notices, "SELL");
    assert_eq!(admin_sell.template_key, "rate_sharp_move");
    assert_eq!(
        admin_sell.event_id,
        format!("rate-sharp-move:{pub2_id}:USD:SELL")
    );
    assert_sharp_move_params(&admin_sell, &pub2_id, MAIN, "USD", "SELL");

    // Assert neither teller nor expired admin nor other organisation admin received notices
    let teller_notices = sharp_move_notifications_for_user(&db.pool, teller_id, &pub2_id).await;
    assert_eq!(teller_notices.len(), 0, "TELLER must not receive notice");
    let expired_notices =
        sharp_move_notifications_for_user(&db.pool, expired_admin_id, &pub2_id).await;
    assert_eq!(
        expired_notices.len(),
        0,
        "expired ADMIN must not receive notice"
    );
    let other_org_notices =
        sharp_move_notifications_for_user(&db.pool, other_org_admin, &pub2_id).await;
    assert_eq!(
        other_org_notices.len(),
        0,
        "ADMIN of another organisation must not receive notice"
    );

    db.finish().await;
}

/// Row 2: A branch-scoped ADMIN is not told (`branch_id IS NULL` is required in `user_roles`).
#[tokio::test]
async fn branch_scoped_admin_is_not_told() {
    let db = TestDb::start().await;

    let org_admin_id = create_user_with_role(&db.pool, "Org Admin", "ADMIN", None, None).await;
    let branch_admin_id =
        create_user_with_role(&db.pool, "Branch Admin", "ADMIN", Some(MAIN), None).await;

    seed_policy(&db.pool, MAIN, 1, Some("2.0000")).await;

    publish_rates(&db.pool, MAIN, &[("USD", "30.0000", "31.0000")]).await;
    let pub2_id = publish_rates(&db.pool, MAIN, &[("USD", "31.0000", "31.0000")]).await;

    // Org-wide admin receives 1 notification (positive control; fails on main)
    let org_notices = sharp_move_notifications_for_user(&db.pool, org_admin_id, &pub2_id).await;
    assert_eq!(
        org_notices.len(),
        1,
        "org-wide ADMIN must receive a notification on sharp move"
    );

    // Branch-scoped admin receives no notification
    let branch_notices =
        sharp_move_notifications_for_user(&db.pool, branch_admin_id, &pub2_id).await;
    assert_eq!(
        branch_notices.len(),
        0,
        "branch-scoped ADMIN must not be told"
    );

    db.finish().await;
}

/// Row 3: Threshold NULL produces no notice (`sharp_move_alert_pct IS NULL`).
#[tokio::test]
async fn threshold_null_produces_no_notice() {
    let db = TestDb::start().await;

    let admin_id = create_user_with_role(&db.pool, "Admin Org", "ADMIN", None, None).await;

    // Step 1: Active policy with 2.0% threshold. Sharp move produces notice (positive control).
    seed_policy(&db.pool, MAIN, 1, Some("2.0000")).await;
    publish_rates(&db.pool, MAIN, &[("USD", "30.0000", "31.0000")]).await;
    let pub2_id = publish_rates(&db.pool, MAIN, &[("USD", "31.0000", "31.0000")]).await;

    let pub2_notices = sharp_move_notifications_for_user(&db.pool, admin_id, &pub2_id).await;
    assert_eq!(
        pub2_notices.len(),
        1,
        "active threshold must produce a notification on sharp move"
    );

    // Step 2: New policy version with sharp_move_alert_pct = NULL. Expire version 1.
    sqlx::query(
        "UPDATE rate_policy_versions SET valid_to = now() WHERE branch_id = $1 AND version = 1",
    )
    .bind(MAIN)
    .execute(&db.pool)
    .await
    .expect("expire version 1");
    seed_policy(&db.pool, MAIN, 2, None).await;

    // Publication 3 moves USD buy from 31.0000 to 32.0000 (3.22% move), sell from 31.0000 to 33.0000 (6.45% move).
    // Because threshold is NULL, it must produce no notice.
    let pub3_id = publish_rates(&db.pool, MAIN, &[("USD", "32.0000", "33.0000")]).await;
    let pub3_notices = sharp_move_notifications_for_pub(&db.pool, &pub3_id).await;
    assert_eq!(
        pub3_notices.len(),
        0,
        "threshold NULL must produce no notification"
    );

    db.finish().await;
}

/// Row 4: Move exactly at the threshold produces no notice (`move_pct > threshold` strictly greater).
#[tokio::test]
async fn move_exactly_at_threshold_produces_no_notice() {
    let db = TestDb::start().await;

    let admin_id = create_user_with_role(&db.pool, "Admin Org", "ADMIN", None, None).await;
    seed_policy(&db.pool, MAIN, 1, Some("2.0000")).await;

    // Baseline pub 1: USD buy 50.0000, sell 100.0000 (buy <= sell)
    publish_rates(&db.pool, MAIN, &[("USD", "50.0000", "100.0000")]).await;

    // Pub 2: USD buy moves from 50.0000 to 51.0000 (sell remains 100.0000).
    // move_pct = |51.0000 - 50.0000| / 50.0000 * 100 = 2.0000%, exactly at threshold 2.0%.
    // Must produce NO notice because the rule is strictly greater (move_pct > threshold).
    let pub2_id = publish_rates(&db.pool, MAIN, &[("USD", "51.0000", "100.0000")]).await;

    // Pub 3: USD buy moves from 51.0000 to 52.5000.
    // move_pct = |52.5000 - 51.0000| / 51.0000 * 100 = 2.941...% > 2.0%.
    // Strictly greater than threshold -> must produce notice (positive control).
    let pub3_id = publish_rates(&db.pool, MAIN, &[("USD", "52.5000", "100.0000")]).await;

    let pub3_notices = sharp_move_notifications_for_user(&db.pool, admin_id, &pub3_id).await;
    assert_eq!(
        pub3_notices.len(),
        1,
        "move strictly greater than threshold must produce a notification"
    );

    let pub2_notices = sharp_move_notifications_for_user(&db.pool, admin_id, &pub2_id).await;
    assert_eq!(
        pub2_notices.len(),
        0,
        "move exactly at threshold must produce no notification"
    );

    db.finish().await;
}

/// Row 6: The publish rolls back -> no notice written (notices inside publish transaction roll back with it).
#[tokio::test]
async fn publish_rollback_writes_no_notice() {
    let db = TestDb::start().await;

    let admin_id = create_user_with_role(&db.pool, "Admin Org", "ADMIN", None, None).await;
    seed_policy(&db.pool, MAIN, 1, Some("2.0000")).await;

    publish_rates(&db.pool, MAIN, &[("USD", "30.0000", "31.0000")]).await;

    // Committed publish with sharp move: positive control, must write notice (fails on main)
    let pub2_id = publish_rates(&db.pool, MAIN, &[("USD", "31.0000", "31.0000")]).await;
    let pub2_notices = sharp_move_notifications_for_user(&db.pool, admin_id, &pub2_id).await;
    assert_eq!(
        pub2_notices.len(),
        1,
        "committed publish with sharp move must write a notification"
    );

    // Pub 3 with sharp move: execute inside transaction and roll back
    let (draft3_id, expected3) =
        create_and_submit_approved_draft(&db.pool, MAIN, &[("USD", "32.0000", "33.0000")]).await;
    let publisher = PublishService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin publish tx");
    let pub3_ref = publisher
        .publish(
            &mut tx,
            &BranchId::new(MAIN),
            &owner_actor(),
            &draft3_id,
            expected3,
        )
        .await
        .expect("publish draft inside transaction");

    // Roll back the publish transaction
    tx.rollback().await.expect("rollback publish transaction");

    let rolled_back_notices =
        sharp_move_notifications_for_pub(&db.pool, &pub3_ref.id.to_string()).await;
    assert_eq!(
        rolled_back_notices.len(),
        0,
        "rolled back publish must write no notice"
    );

    db.finish().await;
}

/// Row 7: A retry of the same publish -> no second notice (`event_id` idempotency `"rate-sharp-move:<pub_id>:<currency>:<side>"`).
/// Re-calling `SharpMoveNotices::notify_in` for the same publication's moves produces no second row and no error.
#[tokio::test]
async fn retry_of_same_publish_produces_no_second_notice() {
    let db = TestDb::start().await;

    let admin_id = create_user_with_role(&db.pool, "Admin Org", "ADMIN", None, None).await;
    seed_policy(&db.pool, MAIN, 1, Some("2.0000")).await;

    publish_rates(&db.pool, MAIN, &[("USD", "30.0000", "31.0000")]).await;

    // Pub 2 with sharp move commits (fails on main)
    let pub2_id = publish_rates(&db.pool, MAIN, &[("USD", "31.0000", "31.0000")]).await;
    let pub2_notices = sharp_move_notifications_for_user(&db.pool, admin_id, &pub2_id).await;
    assert_eq!(
        pub2_notices.len(),
        1,
        "committed publish with sharp move must write a notification"
    );

    // Call notify_in a second time with the same publication's moves
    let b = BranchId::new(MAIN);
    let moves = vec![Move {
        currency: CurrencyCode::parse("USD").unwrap(),
        group: GroupCode::all(),
        side: Side::Buy,
        old: Rate::parse("30.0000").unwrap(),
        new: Rate::parse("31.0000").unwrap(),
        move_pct: Decimal::from_str("3.333333333333333333333333333").unwrap(),
    }];

    let pool = db.pool.clone();
    let notices = SharpMoveNotices::new(Arc::new(NotificationService::new(
        pool.clone(),
        Arc::new(TemplateRegistry::generated()),
    )));

    let mut tx = db.pool.begin().await.expect("begin tx for notify_in retry");
    notices
        .notify_in(&mut tx, &b, &moves)
        .await
        .expect("notify_in retry must not error");
    tx.commit().await.expect("commit notify_in retry tx");

    let after_retry = sharp_move_notifications_for_user(&db.pool, admin_id, &pub2_id).await;
    assert_eq!(
        after_retry.len(),
        1,
        "a retry of notify_in for the same publication must produce no second notice"
    );

    db.finish().await;
}

/// Row 5: Notice-write failure keeps the publication and the other notices.
/// (Lane controller ruling #1584 comment-6137651681: publication commits either way; notices under SAVEPOINT).
#[tokio::test]
async fn notice_write_failure_keeps_the_publication_and_the_other_notices() {
    let db = TestDb::start().await;

    let exec_id = create_user_with_role(&db.pool, "Executive Org", "EXECUTIVE", None, None).await;
    let admin_id = create_user_with_role(&db.pool, "Admin Org", "ADMIN", None, None).await;

    seed_policy(&db.pool, MAIN, 1, Some("2.0000")).await;

    // Test-only trigger: raises an exception for any notification insertion with event_id containing ':USD:'
    sqlx::query(
        "CREATE OR REPLACE FUNCTION fail_usd_notice() RETURNS trigger AS $$ \
         BEGIN \
             IF NEW.event_id LIKE '%:USD:%' THEN \
                 RAISE EXCEPTION 'test: notice-write failure for USD'; \
             END IF; \
             RETURN NEW; \
         END; \
         $$ LANGUAGE plpgsql",
    )
    .execute(&db.pool)
    .await
    .expect("create fail_usd_notice function");

    sqlx::query(
        "CREATE TRIGGER trigger_fail_usd_notice \
         BEFORE INSERT ON notifications \
         FOR EACH ROW \
         EXECUTE FUNCTION fail_usd_notice()",
    )
    .execute(&db.pool)
    .await
    .expect("create trigger_fail_usd_notice");

    // Baseline publication 1: USD 30.0000 / 31.0000, EUR 35.0000 / 38.0000
    publish_rates(
        &db.pool,
        MAIN,
        &[("USD", "30.0000", "31.0000"), ("EUR", "35.0000", "38.0000")],
    )
    .await;

    // Publication 2: Both USD and EUR move sharply
    // USD buy: 30.0000 -> 31.0000 (3.33% move > 2%)
    // EUR buy: 35.0000 -> 36.5000 (4.28% move > 2%, sell remains 38.0000 so buy <= sell)
    let pub2_id = publish_rates(
        &db.pool,
        MAIN,
        &[("USD", "31.0000", "31.0000"), ("EUR", "36.5000", "38.0000")],
    )
    .await;

    // 1. Assert the publication row is committed
    let pub_count: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_publications WHERE id = $1")
        .bind(Uuid::parse_str(&pub2_id).unwrap())
        .fetch_one(&db.pool)
        .await
        .expect("check publication row committed");
    assert_eq!(pub_count, 1, "the publication row must be committed");

    // 2. Assert there is no USD rate_sharp_move notice
    let usd_notices: Vec<NotificationRow> = sqlx::query_as(
        "SELECT id, organization_id, recipient_user_id, event_id, template_key, \
                template_version, object_type, object_id, params \
         FROM notifications \
         WHERE template_key = 'rate_sharp_move' AND object_id = $1 AND event_id LIKE '%:USD:%'",
    )
    .bind(&pub2_id)
    .fetch_all(&db.pool)
    .await
    .expect("query USD notices");
    assert_eq!(
        usd_notices.len(),
        0,
        "there must be no USD rate_sharp_move notice"
    );

    // 3. Assert the EUR notices exist for every recipient (one for executive, one for admin)
    let eur_notices: Vec<NotificationRow> = sqlx::query_as(
        "SELECT id, organization_id, recipient_user_id, event_id, template_key, \
                template_version, object_type, object_id, params \
         FROM notifications \
         WHERE template_key = 'rate_sharp_move' AND object_id = $1 AND event_id LIKE '%:EUR:%' \
         ORDER BY recipient_user_id",
    )
    .bind(&pub2_id)
    .fetch_all(&db.pool)
    .await
    .expect("query EUR notices");
    assert_eq!(
        eur_notices.len(),
        2,
        "EUR notices must exist for every recipient (found {})",
        eur_notices.len()
    );

    let exec_eur = sharp_move_notifications_for_user(&db.pool, exec_id, &pub2_id).await;
    assert_eq!(exec_eur.len(), 1, "executive must receive EUR notice");
    assert!(exec_eur[0].event_id.contains(":EUR:"));

    let admin_eur = sharp_move_notifications_for_user(&db.pool, admin_id, &pub2_id).await;
    assert_eq!(admin_eur.len(), 1, "admin must receive EUR notice");
    assert!(admin_eur[0].event_id.contains(":EUR:"));

    db.finish().await;
}

/// Row 8: Rates with 4 or more integer digits (USD 1000 -> 1040, +4%, threshold 2%) publish successfully and notify.
/// Under the amended contract, params contain only {branch, currency, side}, avoiding any 4+ digit numeric strings
/// that NotificationRequest::new / redact_text would reject.
#[tokio::test]
async fn four_digit_rates_sharp_move_commits_and_notifies_org_executive() {
    let db = TestDb::start().await;

    let exec_id = create_user_with_role(&db.pool, "Executive Org", "EXECUTIVE", None, None).await;
    seed_policy(&db.pool, MAIN, 1, Some("2.0000")).await;

    // Baseline publication 1: USD 1000.0000 / 1100.0000
    publish_rates(&db.pool, MAIN, &[("USD", "1000.0000", "1100.0000")]).await;

    // Publication 2: USD 1040.0000 / 1144.0000 (+4% move > 2% threshold)
    let pub2_id = publish_rates(&db.pool, MAIN, &[("USD", "1040.0000", "1144.0000")]).await;

    // The second publish must commit
    let pub_count: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_publications WHERE id = $1")
        .bind(Uuid::parse_str(&pub2_id).unwrap())
        .fetch_one(&db.pool)
        .await
        .expect("check publication row committed");
    assert_eq!(pub_count, 1, "the second publication row must be committed");

    // The org-wide EXECUTIVE must get the notice
    let exec_notices = sharp_move_notifications_for_user(&db.pool, exec_id, &pub2_id).await;
    assert_eq!(
        exec_notices.len(),
        2,
        "org-wide EXECUTIVE must receive 2 notifications (BUY and SELL)"
    );

    let find_notice = |notices: &[NotificationRow], side: &str| -> NotificationRow {
        notices
            .iter()
            .find(|n| {
                n.params.get("side").and_then(Value::as_str) == Some(side)
                    || n.event_id.ends_with(&format!(":{side}"))
            })
            .cloned()
            .unwrap_or_else(|| panic!("notice for side {side} not found in {notices:?}"))
    };

    let exec_buy = find_notice(&exec_notices, "BUY");
    assert_eq!(exec_buy.template_key, "rate_sharp_move");
    assert_eq!(
        exec_buy.event_id,
        format!("rate-sharp-move:{pub2_id}:USD:BUY")
    );
    assert_sharp_move_params(&exec_buy, &pub2_id, MAIN, "USD", "BUY");

    let exec_sell = find_notice(&exec_notices, "SELL");
    assert_eq!(exec_sell.template_key, "rate_sharp_move");
    assert_eq!(
        exec_sell.event_id,
        format!("rate-sharp-move:{pub2_id}:USD:SELL")
    );
    assert_sharp_move_params(&exec_sell, &pub2_id, MAIN, "USD", "SELL");

    db.finish().await;
}
