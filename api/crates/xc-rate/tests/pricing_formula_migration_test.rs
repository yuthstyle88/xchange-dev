//! pricing-formula migration (docs/design/blueprint/pricing-formula.md §8; approved as Q-F1 + Q-F3, §10): the
//! `pricing_formula_versions` table, the branch formula pair on `rate_policy_versions`, and `formula_source` /
//! `formula_version_id` on `rate_draft_lines` and `pricing_tier_sets`.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Needs `TEST_DATABASE_URL` (a Postgres 17 database whose name ends in `_test`). Every test applies every migration
//! of `api/migrations/` to a fresh schema.

mod rate_engine_common;

use rate_engine_common::TestDb;
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_rate::application::policy::policy_version_writer::{
    PolicyVersionChange, PolicyVersionWriter,
};

const SEEDED_ORG: &str = "00000000-0000-0000-0000-000000000001";

fn org() -> Uuid {
    Uuid::parse_str(SEEDED_ORG).expect("the seeded organisation id")
}

async fn insert_formula(
    db: &TestDb,
    version: i32,
    buy: &str,
    sell: &str,
    current: bool,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pricing_formula_versions \
         (id, organization_id, version, buy_formula, sell_formula, valid_from, valid_to, approved_by, reason, source_version) \
         VALUES ($1, $2, $3, $4, $5, now(), CASE WHEN $6 THEN NULL ELSE now() END, 'exec-1', 'test', NULL)",
    )
    .bind(id)
    .bind(org())
    .bind(version)
    .bind(buy)
    .bind(sell)
    .bind(current)
    .execute(&db.pool)
    .await
    .map(|_| id)
}

/// The SQLSTATE of a refused statement.
fn state(e: sqlx::Error) -> String {
    e.as_database_error()
        .and_then(|d| d.code().map(|c| c.into_owned()))
        .unwrap_or_else(|| panic!("a database error, got {e}"))
}

const UNIQUE: &str = "23505";
const CHECK: &str = "23514";
const FOREIGN_KEY: &str = "23503";

#[tokio::test]
async fn organisation_formula_versions_are_stored_one_current_per_organisation() {
    let db = TestDb::start().await;
    insert_formula(&db, 1, "ref_mid - 0.05", "ref_mid + 0.05", false)
        .await
        .expect("version 1 (closed)");
    insert_formula(&db, 2, "ref_buy", "ref_sell", true)
        .await
        .expect("version 2 (current)");
    let second_current = insert_formula(&db, 3, "ref_buy", "ref_sell", true)
        .await
        .expect_err("a second current version is refused");
    assert_eq!(state(second_current), UNIQUE);
    let same_number = insert_formula(&db, 2, "ref_buy", "ref_sell", false)
        .await
        .expect_err("a repeated version number is refused");
    assert_eq!(state(same_number), UNIQUE);
}

#[tokio::test]
async fn formula_text_is_1_to_500_characters() {
    let db = TestDb::start().await;
    let empty = insert_formula(&db, 1, "", "ref_sell", true)
        .await
        .expect_err("an empty buy formula is refused");
    assert_eq!(state(empty), CHECK);
    let long = "1".repeat(501);
    let too_long = insert_formula(&db, 1, "ref_buy", &long, true)
        .await
        .expect_err("a 501-character sell formula is refused");
    assert_eq!(state(too_long), CHECK);
    insert_formula(&db, 1, &"1".repeat(500), "ref_sell", true)
        .await
        .expect("500 characters are stored");
}

#[tokio::test]
async fn an_organisation_formula_needs_a_real_organisation() {
    let db = TestDb::start().await;
    let refused = sqlx::query(
        "INSERT INTO pricing_formula_versions \
         (id, organization_id, version, buy_formula, sell_formula, valid_from, approved_by, reason) \
         VALUES ($1, $2, 1, 'ref_buy', 'ref_sell', now(), 'exec-1', 'test')",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(&db.pool)
    .await
    .expect_err("an unknown organisation is refused");
    assert_eq!(state(refused), FOREIGN_KEY);
}

#[tokio::test]
async fn a_branch_formula_is_both_texts_or_neither() {
    let db = TestDb::start().await;
    db.seed_branch("PF1").await;
    let mut tx = db.begin().await;
    let written = PolicyVersionWriter::next(
        &mut tx,
        &BranchId::new("PF1"),
        PolicyVersionChange {
            buy_adjust_pct: Decimal::new(3, 1),
            sell_adjust_pct: Decimal::new(-3, 1),
            guards: json!({}),
            max_reference_age_hours: None,
            on_stale: None,
            stale_limit_thb: None,
            value_tiers_on: false,
            sharp_move_alert_pct: None,
            value_tiers: Vec::new(),
            anchor_source: None,
            overrides: None,
            approved_by: "exec-1".into(),
            reason: "migration test".into(),
        },
    )
    .await
    .expect("a policy version");
    tx.commit().await.expect("commit");

    let row =
        sqlx::query("SELECT buy_formula, sell_formula FROM rate_policy_versions WHERE id = $1")
            .bind(written.id)
            .fetch_one(&db.pool)
            .await
            .expect("the version");
    let (buy, sell): (Option<String>, Option<String>) = (row.get(0), row.get(1));
    assert_eq!(
        (buy, sell),
        (None, None),
        "an existing-style version follows the organisation"
    );

    let one_side =
        sqlx::query("UPDATE rate_policy_versions SET buy_formula = 'ref_buy' WHERE id = $1")
            .bind(written.id)
            .execute(&db.pool)
            .await
            .expect_err("one text alone is refused");
    assert_eq!(state(one_side), CHECK);

    sqlx::query(
        "UPDATE rate_policy_versions SET buy_formula = 'ref_mid - 0.05', sell_formula = 'ref_mid + 0.05' \
         WHERE id = $1",
    )
    .bind(written.id)
    .execute(&db.pool)
    .await
    .expect("both texts are stored");
}

/// `(data_type, is_nullable)` of a column, or `None` when it does not exist.
async fn column(db: &TestDb, table: &str, name: &str) -> Option<(String, String)> {
    sqlx::query(
        "SELECT data_type, is_nullable FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2",
    )
    .bind(table)
    .bind(name)
    .fetch_optional(&db.pool)
    .await
    .expect("read information_schema")
    .map(|r| (r.get(0), r.get(1)))
}

/// Every CHECK and FOREIGN KEY definition on `table`.
async fn constraints(db: &TestDb, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT pg_get_constraintdef(c.oid) FROM pg_constraint c \
         JOIN pg_class t ON t.oid = c.conrelid JOIN pg_namespace n ON n.oid = t.relnamespace \
         WHERE n.nspname = current_schema() AND t.relname = $1 AND c.contype IN ('c', 'f')",
    )
    .bind(table)
    .fetch_all(&db.pool)
    .await
    .expect("read pg_constraint")
}

#[tokio::test]
async fn draft_lines_and_tier_sets_record_the_formula_source() {
    let db = TestDb::start().await;
    for table in ["rate_draft_lines", "pricing_tier_sets"] {
        assert_eq!(
            column(&db, table, "formula_source").await,
            Some(("text".into(), "YES".into())),
            "{table}.formula_source: nullable text (old rows read as before formulas)"
        );
        assert_eq!(
            column(&db, table, "formula_version_id").await,
            Some(("uuid".into(), "YES".into())),
            "{table}.formula_version_id: nullable uuid"
        );
        let defs = constraints(&db, table).await;
        assert!(
            defs.iter().any(|d| d.contains("formula_source")
                && d.contains("'BRANCH'")
                && d.contains("'ORGANISATION'")
                && d.contains("'DEFAULT'")),
            "{table}: a CHECK limits formula_source to BRANCH, ORGANISATION, DEFAULT: {defs:?}"
        );
        assert!(
            defs.iter()
                .any(|d| d.contains("FOREIGN KEY (formula_version_id)")
                    && d.contains("REFERENCES pricing_formula_versions(id)")),
            "{table}: formula_version_id references pricing_formula_versions: {defs:?}"
        );
    }
}
