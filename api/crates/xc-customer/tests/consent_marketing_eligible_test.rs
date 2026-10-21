//! Up-front tests for `ConsentService::marketing_eligible_in` (design `customer-identity-consent.md` §4.3, §5, §5.1;
//! marketing-campaigns D-13).
//!
//! A customer is marketing-eligible when the newest `MARKETING` row (by `effective_at DESC, created_at DESC`) is
//! `granted` on at least one of SMS, EMAIL, PUSH. One query answers the whole list; an empty list answers an empty
//! set without a query. One test per behaviour row of the ticket.

use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_customer::application::consent_service::ConsentService;
use xc_customer::domain::consent::Channel;

const ORG_ID_STR: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL must be set, e.g. postgres://postgres:postgres@127.0.0.1:55435/xchange_test")
    })
}

async fn setup_db() -> (PgPool, String) {
    let db_url = url();
    let schema = unique("consent_elig");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .expect("admin connect");

    sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create schema");
    admin.close().await;

    let options: sqlx::postgres::PgConnectOptions = db_url.parse().unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect schema pool");

    let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let mut entries: Vec<_> = std::fs::read_dir(&migrations_dir)
        .unwrap_or_else(|e| panic!("read migrations dir {}: {e}", migrations_dir.display()))
        .map(|r| r.expect("read entry").path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    entries.sort();

    for path in entries {
        let sql = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        sqlx::raw_sql(AssertSqlSafe(sql))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("failed to execute {}: {e}", path.display()));
    }

    (pool, schema)
}

async fn cleanup_db(pool: PgPool, schema: String) {
    let db_url = url();
    pool.close().await;

    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .expect("admin reconnect");

    sqlx::raw_sql(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .expect("drop schema");
    admin.close().await;
}

/// Whole seconds, so a round trip through `timestamptz` is exact on every platform.
fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_760_371_200 + secs, 0).expect("valid timestamp")
}

fn org() -> Uuid {
    Uuid::parse_str(ORG_ID_STR).unwrap()
}

async fn seed_customer(pool: &PgPool) -> String {
    let customer_id = unique("CX");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2, $1, $3, 'LIVE', $4)",
    )
    .bind(&customer_id)
    .bind(org())
    .bind("Consent Test")
    .bind(at(-86_400))
    .execute(pool)
    .await
    .expect("insert customer");
    customer_id
}

/// One consent row with full control over `effective_at` and `created_at` (the service's own writers set
/// `created_at = effective_at`, which cannot express the tie-break rows).
async fn seed_consent(
    pool: &PgPool,
    customer_id: &str,
    purpose: &str,
    channel: Option<&str>,
    granted: bool,
    effective_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO consents ( \
             id, organization_id, customer_id, purpose, channel, policy_version, \
             lang, granted, effective_at, evidence_ref, recorded_by, created_at \
         ) VALUES ($1, $2, $3, $4, $5, 1, 'en', $6, $7, 'test-evidence', NULL, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(org())
    .bind(customer_id)
    .bind(purpose)
    .bind(channel)
    .bind(granted)
    .bind(effective_at)
    .bind(created_at)
    .execute(pool)
    .await
    .expect("insert consent");
}

/// `MARKETING` row whose `created_at` equals `effective_at`.
async fn marketing(pool: &PgPool, customer_id: &str, channel: &str, granted: bool, t: i64) {
    seed_consent(
        pool,
        customer_id,
        "MARKETING",
        Some(channel),
        granted,
        at(t),
        at(t),
    )
    .await;
}

async fn eligible(pool: &PgPool, ids: &[String]) -> HashSet<String> {
    let mut tx = pool.begin().await.expect("begin");
    let set = ConsentService::marketing_eligible_in(&mut tx, ids)
        .await
        .expect("marketing_eligible_in");
    tx.rollback().await.expect("rollback");
    set
}

/// Sequential + index scans on this schema's `consents` counted inside the current transaction.
async fn consents_scans(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> i64 {
    sqlx::query_scalar(
        "SELECT (seq_scan + coalesce(idx_scan, 0))::bigint FROM pg_stat_xact_user_tables \
         WHERE relname = 'consents' AND schemaname = current_schema()",
    )
    .fetch_one(&mut **tx)
    .await
    .expect("read pg_stat_xact_user_tables")
}

// Row 1
#[test]
fn granted_on_exactly_one_channel_is_eligible() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let c = seed_customer(&pool).await;
        marketing(&pool, &c, "EMAIL", true, 0).await;

        let set = eligible(&pool, std::slice::from_ref(&c)).await;
        assert!(
            set.contains(&c),
            "EMAIL-only grant must be eligible: {set:?}"
        );
        assert_eq!(set.len(), 1);

        cleanup_db(pool, schema).await;
    });
}

// Row 2
#[test]
fn newest_row_per_channel_wins_withdrawn_later_is_not_eligible() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let c = seed_customer(&pool).await;
        marketing(&pool, &c, "SMS", true, 0).await;
        marketing(&pool, &c, "SMS", false, 60).await;

        let set = eligible(&pool, std::slice::from_ref(&c)).await;
        assert!(
            !set.contains(&c),
            "SMS granted then withdrawn later must not be eligible: {set:?}"
        );

        cleanup_db(pool, schema).await;
    });
}

// Row 3
#[test]
fn granted_after_withdrawal_on_same_channel_is_eligible() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let c = seed_customer(&pool).await;
        marketing(&pool, &c, "PUSH", false, 0).await;
        marketing(&pool, &c, "PUSH", true, 60).await;

        let set = eligible(&pool, std::slice::from_ref(&c)).await;
        assert!(
            set.contains(&c),
            "PUSH withdrawn then granted later must be eligible: {set:?}"
        );

        cleanup_db(pool, schema).await;
    });
}

/// (channel, rows as (granted, created_at offset) in insertion order, expected eligible).
type TieCase = (&'static str, [(bool, i64); 2], bool);

// Row 4 — both directions of the created_at tie-break, each seeded in both insertion orders so that no
// positional pick among the tied rows (first or last physical row) can pass all four customers.
#[test]
fn same_effective_at_later_created_at_wins_both_directions() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;

        // (granted, created_at offset) per row, in insertion order.
        let cases: [TieCase; 4] = [
            ("SMS", [(true, 5), (false, 10)], false), // withdrawal created later, inserted last
            ("SMS", [(false, 10), (true, 5)], false), // withdrawal created later, inserted first
            ("EMAIL", [(false, 5), (true, 10)], true), // grant created later, inserted last
            ("EMAIL", [(true, 10), (false, 5)], true), // grant created later, inserted first
        ];

        let mut seeded = Vec::new();
        for (channel, rows, expected) in cases {
            let c = seed_customer(&pool).await;
            for (granted, created) in rows {
                seed_consent(
                    &pool,
                    &c,
                    "MARKETING",
                    Some(channel),
                    granted,
                    at(0),
                    at(created),
                )
                .await;
            }
            seeded.push((c, rows, expected));
        }

        let ids: Vec<String> = seeded.iter().map(|(c, _, _)| c.clone()).collect();
        let set = eligible(&pool, &ids).await;
        for (c, rows, expected) in &seeded {
            assert_eq!(
                set.contains(c),
                *expected,
                "same effective_at: the row created later must win (rows in insertion order {rows:?}): {set:?}"
            );
        }

        cleanup_db(pool, schema).await;
    });
}

// Row 5
#[test]
fn customer_with_no_consent_rows_is_not_eligible() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let c = seed_customer(&pool).await;

        let set = eligible(&pool, std::slice::from_ref(&c)).await;
        assert!(set.is_empty(), "no consent rows ⇒ empty set: {set:?}");

        cleanup_db(pool, schema).await;
    });
}

// Row 6
#[test]
fn only_non_marketing_purpose_granted_is_not_eligible() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let c = seed_customer(&pool).await;
        // TERMS rows carry a NULL channel in production; a TERMS row with a channel is also not MARKETING.
        seed_consent(&pool, &c, "TERMS", None, true, at(0), at(0)).await;
        seed_consent(&pool, &c, "TERMS", Some("SMS"), true, at(1), at(1)).await;

        let set = eligible(&pool, std::slice::from_ref(&c)).await;
        assert!(
            !set.contains(&c),
            "only TERMS granted must not be marketing-eligible: {set:?}"
        );

        cleanup_db(pool, schema).await;
    });
}

// Row 7 — the schema allows a NULL channel (`channel text NULL CHECK (channel IN ('SMS','EMAIL','PUSH'))`,
// migrations/20261019100000_customers_org.sql), and the purpose CHECK does not tie NULL to TERMS.
#[test]
fn marketing_row_with_null_channel_is_ignored() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let c = seed_customer(&pool).await;
        seed_consent(&pool, &c, "MARKETING", None, true, at(0), at(0)).await;

        let set = eligible(&pool, std::slice::from_ref(&c)).await;
        assert!(
            !set.contains(&c),
            "a MARKETING grant with a NULL channel names no channel and must be ignored: {set:?}"
        );

        cleanup_db(pool, schema).await;
    });
}

// Row 8 — empty input answers an empty set, and does not touch `consents` at all.
#[test]
fn empty_list_answers_empty_set_without_a_query() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let c = seed_customer(&pool).await;
        marketing(&pool, &c, "SMS", true, 0).await;

        let mut tx = pool.begin().await.expect("begin");
        let before = consents_scans(&mut tx).await;
        let set = ConsentService::marketing_eligible_in(&mut tx, &[])
            .await
            .expect("marketing_eligible_in");
        let after = consents_scans(&mut tx).await;
        tx.rollback().await.expect("rollback");

        assert!(set.is_empty(), "empty list ⇒ empty set: {set:?}");
        assert_eq!(after - before, 0, "empty list must not read consents");

        cleanup_db(pool, schema).await;
    });
}

// Row 9
#[test]
fn only_ids_from_the_list_are_returned_and_duplicates_are_fine() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let in_list = seed_customer(&pool).await;
        let outsider = seed_customer(&pool).await;
        marketing(&pool, &in_list, "SMS", true, 0).await;
        marketing(&pool, &outsider, "SMS", true, 0).await;

        let ids = vec![in_list.clone(), in_list.clone(), unique("CX_UNKNOWN")];
        let set = eligible(&pool, &ids).await;
        let expected: HashSet<String> = [in_list.clone()].into_iter().collect();
        assert_eq!(
            set, expected,
            "only listed, eligible ids are returned; an eligible customer outside the list is not"
        );

        cleanup_db(pool, schema).await;
    });
}

/// 50 customers, one of eight consent shapes each (`i % 8`). Answers the ids in seed order.
async fn seed_mixed(pool: &PgPool, n: usize) -> Vec<String> {
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let c = seed_customer(pool).await;
        match i % 8 {
            0 => {}                                         // no rows
            1 => marketing(pool, &c, "SMS", true, 0).await, // single grant
            2 => {
                marketing(pool, &c, "EMAIL", true, 0).await;
                marketing(pool, &c, "EMAIL", false, 60).await; // withdrawn later
            }
            3 => {
                marketing(pool, &c, "PUSH", false, 0).await;
                marketing(pool, &c, "PUSH", true, 60).await; // re-granted later
            }
            4 => {
                marketing(pool, &c, "SMS", false, 0).await;
                marketing(pool, &c, "EMAIL", false, 0).await;
                marketing(pool, &c, "PUSH", true, 0).await; // one of three
            }
            5 => seed_consent(pool, &c, "TERMS", None, true, at(0), at(0)).await,
            6 => {
                seed_consent(pool, &c, "MARKETING", Some("SMS"), false, at(0), at(10)).await;
                seed_consent(pool, &c, "MARKETING", Some("SMS"), true, at(0), at(5)).await;
            }
            _ => {
                marketing(pool, &c, "SMS", true, 0).await;
                marketing(pool, &c, "SMS", false, 30).await;
                marketing(pool, &c, "EMAIL", false, 0).await;
            }
        }
        ids.push(c);
    }
    ids
}

// Row 10 — one query for the whole list, whatever its length.
#[test]
fn one_query_regardless_of_count() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let ids = seed_mixed(&pool, 50).await;

        let mut tx = pool.begin().await.expect("begin");
        let before = consents_scans(&mut tx).await;
        let set = ConsentService::marketing_eligible_in(&mut tx, &ids)
            .await
            .expect("marketing_eligible_in");
        let after = consents_scans(&mut tx).await;
        tx.rollback().await.expect("rollback");

        // `consents` has no index other than its primary key, so one `customer_id = ANY($1)` query is
        // exactly one sequential scan (measured: 1). The per-customer × per-channel loop it replaces is
        // 3 × 50 = 150. `== 1`, not `<= 1`: a reading of 0 would mean the counter is not tracking (e.g.
        // `track_counts = off`), and this test would then prove nothing.
        let scans = after - before;
        assert_eq!(
            scans, 1,
            "marketing_eligible_in must read consents with one query for 50 customers; saw {scans} scans"
        );
        assert!(!set.is_empty(), "the seed has eligible customers");

        cleanup_db(pool, schema).await;
    });
}

// Row 11 — the batched answer equals SMS ‖ EMAIL ‖ PUSH from the existing per-channel rule.
#[test]
fn equivalent_to_per_channel_marketing_allowed_in() {
    sqlx::test_block_on(async {
        let (pool, schema) = setup_db().await;
        let ids = seed_mixed(&pool, 50).await;
        let service = ConsentService::pg(pool.clone());

        let mut tx = pool.begin().await.expect("begin");
        let set = ConsentService::marketing_eligible_in(&mut tx, &ids)
            .await
            .expect("marketing_eligible_in");

        let mut expected_eligible = 0;
        for id in &ids {
            let mut any = false;
            for ch in [Channel::Sms, Channel::Email, Channel::Push] {
                any |= service
                    .marketing_allowed_in(&mut tx, id, ch)
                    .await
                    .expect("marketing_allowed_in");
            }
            if any {
                expected_eligible += 1;
            }
            assert_eq!(
                set.contains(id),
                any,
                "{id}: batched eligibility must equal SMS || EMAIL || PUSH"
            );
        }
        tx.rollback().await.expect("rollback");

        assert_eq!(set.len(), expected_eligible, "no extra ids in the set");
        assert!(expected_eligible > 0 && expected_eligible < ids.len());

        cleanup_db(pool, schema).await;
    });
}
