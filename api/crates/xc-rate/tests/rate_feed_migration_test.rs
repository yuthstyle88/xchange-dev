//! Rate feed F2 — the migration of `docs/design/rate-feed-connector.md` §4.
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`); every `api/migrations/` file is
//! applied to a fresh schema, so the new migration must name tables without a schema prefix.
//!
//! What this pins (design §4):
//! - `rate_reference_batches.source_published_at timestamptz NULL`.
//! - `rate_reference_lines`: `instrument_key text`, `value_tier text`,
//!   `tier_lower_exclusive_thb` / `tier_upper_inclusive_thb numeric(20,2)`,
//!   `base_buy` / `base_sell numeric(20,8)`, all NULL-able; a line's identity is
//!   `(batch_id, currency_code, reference_type, COALESCE(instrument_key,''), COALESCE(value_tier,''))`,
//!   so an existing BoT line (both NULL) is as unique as before.
//! - `published_rate_lines`: `value_tier text NOT NULL DEFAULT 'ALL'`, the two tier bounds
//!   `numeric(20,2) NULL`, `base_buy_rate` / `base_sell_rate numeric(20,8) NULL`; its unique key is
//!   `(publication_id, currency_code, denomination_group, value_tier)` and the old three-column key
//!   is gone.
//! - `branches.price_source text NOT NULL DEFAULT 'BOT_DRAFT'`, only `'BOT_DRAFT'` or `'RATE_BOARD'`.
//! - `denomination_group_sources(currency_code FK currencies, group_code, provider, instrument_key,
//!   updated_by, updated_at)`, primary key `(currency_code, group_code, provider)`.

mod rate_engine_common;

use rate_engine_common::*;

/// `(data_type, numeric_precision, numeric_scale, is_nullable, column_default)` of a column, or
/// `None` when the column does not exist.
async fn column(
    db: &TestDb,
    table: &str,
    name: &str,
) -> Option<(String, Option<i32>, Option<i32>, String, Option<String>)> {
    sqlx::query_as(
        "SELECT data_type, numeric_precision, numeric_scale, is_nullable, column_default \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2",
    )
    .bind(table)
    .bind(name)
    .fetch_optional(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("read {table}.{name}: {e}"))
}

async fn assert_numeric(db: &TestDb, table: &str, name: &str, precision: i32, scale: i32) {
    let c = column(db, table, name)
        .await
        .unwrap_or_else(|| panic!("{table}.{name} exists"));
    assert_eq!(c.0, "numeric", "{table}.{name} type");
    assert_eq!(
        (c.1, c.2),
        (Some(precision), Some(scale)),
        "{table}.{name} precision, scale"
    );
    assert_eq!(c.3, "YES", "{table}.{name} is NULL-able");
}

async fn assert_nullable_text(db: &TestDb, table: &str, name: &str) {
    let c = column(db, table, name)
        .await
        .unwrap_or_else(|| panic!("{table}.{name} exists"));
    assert_eq!(c.0, "text", "{table}.{name} type");
    assert_eq!(c.3, "YES", "{table}.{name} is NULL-able");
}

/// The definitions of every UNIQUE / PRIMARY KEY constraint and unique index of `table`.
async fn unique_definitions(db: &TestDb, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT pg_get_indexdef(i.indexrelid) \
         FROM pg_index i \
         JOIN pg_class t ON t.oid = i.indrelid \
         JOIN pg_namespace n ON n.oid = t.relnamespace \
         WHERE n.nspname = current_schema() AND t.relname = $1 AND i.indisunique",
    )
    .bind(table)
    .fetch_all(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("unique indexes of {table}: {e}"))
}

fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

#[tokio::test]
async fn reference_batches_and_lines_gain_the_board_columns() {
    let db = TestDb::start().await;

    let c = column(&db, "rate_reference_batches", "source_published_at")
        .await
        .expect("rate_reference_batches.source_published_at exists");
    assert_eq!(c.0, "timestamp with time zone");
    assert_eq!(c.3, "YES");

    assert_nullable_text(&db, "rate_reference_lines", "instrument_key").await;
    assert_nullable_text(&db, "rate_reference_lines", "value_tier").await;
    assert_numeric(
        &db,
        "rate_reference_lines",
        "tier_lower_exclusive_thb",
        20,
        2,
    )
    .await;
    assert_numeric(
        &db,
        "rate_reference_lines",
        "tier_upper_inclusive_thb",
        20,
        2,
    )
    .await;
    assert_numeric(&db, "rate_reference_lines", "base_buy", 20, 8).await;
    assert_numeric(&db, "rate_reference_lines", "base_sell", 20, 8).await;
    db.finish().await;
}

async fn insert_line(
    db: &TestDb,
    batch: &str,
    key: Option<&str>,
    tier: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO rate_reference_lines \
           (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw, instrument_key, value_tier) \
         VALUES (gen_random_uuid(), $1::uuid, 'USD', 'RATE_BOARD', 1, 32.1, 33.0, '{}'::jsonb, $2, $3)",
    )
    .bind(batch)
    .bind(key)
    .bind(tier)
    .execute(&db.pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn a_reference_line_is_identified_by_its_board_row_and_tier() {
    let db = TestDb::start().await;
    let batch = db.seed_batch(chrono::Utc::now(), &[]).await;

    // Bands and tiers of one currency are separate lines.
    insert_line(&db, &batch, Some("USD"), Some("RETAIL"))
        .await
        .expect("USD / RETAIL");
    insert_line(&db, &batch, Some("USD"), Some("STANDARD"))
        .await
        .expect("USD / STANDARD");
    insert_line(&db, &batch, Some("USD1"), Some("RETAIL"))
        .await
        .expect("USD1 / RETAIL");
    assert!(
        insert_line(&db, &batch, Some("USD"), Some("RETAIL"))
            .await
            .is_err(),
        "the same board row and tier twice in one batch is refused"
    );

    // A line without a key or a tier (every BoT line) is as unique as before: one per
    // (batch, currency, type).
    insert_line(&db, &batch, None, None)
        .await
        .expect("the first keyless line");
    assert!(
        insert_line(&db, &batch, None, None).await.is_err(),
        "a second keyless line of the same currency and type is refused (COALESCE identity)"
    );
    db.finish().await;
}

#[tokio::test]
async fn published_lines_gain_a_tier_and_their_key_includes_it() {
    let db = TestDb::start().await;

    let c = column(&db, "published_rate_lines", "value_tier")
        .await
        .expect("published_rate_lines.value_tier exists");
    assert_eq!(c.0, "text");
    assert_eq!(c.3, "NO", "value_tier is NOT NULL");
    assert!(
        c.4.as_deref().unwrap_or_default().contains("'ALL'"),
        "value_tier defaults to 'ALL', got {:?}",
        c.4
    );
    assert_numeric(
        &db,
        "published_rate_lines",
        "tier_lower_exclusive_thb",
        20,
        2,
    )
    .await;
    assert_numeric(
        &db,
        "published_rate_lines",
        "tier_upper_inclusive_thb",
        20,
        2,
    )
    .await;
    assert_numeric(&db, "published_rate_lines", "base_buy_rate", 20, 8).await;
    assert_numeric(&db, "published_rate_lines", "base_sell_rate", 20, 8).await;

    let keys: Vec<String> = unique_definitions(&db, "published_rate_lines")
        .await
        .iter()
        .map(|d| squash(d))
        .collect();
    assert!(
        keys.iter()
            .any(|d| d.ends_with("(publication_id,currency_code,denomination_group,value_tier)")),
        "a unique key on (publication_id, currency_code, denomination_group, value_tier); found {keys:?}"
    );
    assert!(
        !keys
            .iter()
            .any(|d| d.ends_with("(publication_id,currency_code,denomination_group)")),
        "the old three-column key is dropped; found {keys:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn branches_gain_a_price_source_that_defaults_to_bot_drafts() {
    let db = TestDb::start().await;
    db.seed_branch("F2A").await;

    let source: String = sqlx::query_scalar("SELECT price_source FROM branches WHERE id = 'F2A'")
        .fetch_one(&db.pool)
        .await
        .expect("branches.price_source exists");
    assert_eq!(
        source, "BOT_DRAFT",
        "a branch keeps today's path unless switched"
    );

    sqlx::query("UPDATE branches SET price_source = 'RATE_BOARD' WHERE id = 'F2A'")
        .execute(&db.pool)
        .await
        .expect("RATE_BOARD is allowed");
    assert!(
        sqlx::query("UPDATE branches SET price_source = 'SOMETHING' WHERE id = 'F2A'")
            .execute(&db.pool)
            .await
            .is_err(),
        "any other value is refused"
    );
    assert!(
        sqlx::query("UPDATE branches SET price_source = NULL WHERE id = 'F2A'")
            .execute(&db.pool)
            .await
            .is_err(),
        "price_source is NOT NULL"
    );
    db.finish().await;
}

async fn map_group(
    db: &TestDb,
    currency: &str,
    group: &str,
    provider: &str,
    key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO denomination_group_sources \
           (currency_code, group_code, provider, instrument_key, updated_by) \
         VALUES ($1, $2, $3, $4, 'OWNER')",
    )
    .bind(currency)
    .bind(group)
    .bind(provider)
    .bind(key)
    .execute(&db.pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn a_group_maps_to_one_board_row_per_provider() {
    let db = TestDb::start().await;

    map_group(&db, "USD", "LARGE", "RATE_BOARD", "USD")
        .await
        .expect("USD / LARGE → USD");
    map_group(&db, "USD", "SMALL", "RATE_BOARD", "USD1")
        .await
        .expect("USD / SMALL → USD1");
    map_group(&db, "USD", "LARGE", "OTHER", "X")
        .await
        .expect("another provider maps the same group");
    assert!(
        map_group(&db, "USD", "LARGE", "RATE_BOARD", "USD2")
            .await
            .is_err(),
        "one mapping per (currency, group, provider)"
    );
    assert!(
        map_group(&db, "ZZZ", "ALL", "RATE_BOARD", "ZZZ")
            .await
            .is_err(),
        "the currency must exist"
    );

    let updated_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT updated_at FROM denomination_group_sources \
         WHERE currency_code = 'USD' AND group_code = 'LARGE' AND provider = 'RATE_BOARD'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read updated_at");
    assert!(updated_at.is_some(), "updated_at defaults to now()");
    db.finish().await;
}
