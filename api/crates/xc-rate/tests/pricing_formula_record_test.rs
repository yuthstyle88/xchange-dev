//! pricing-formula wave 2 (docs/design/blueprint/pricing-formula.md §3 "Audit", §3a, §8; B15, B22):
//! `FormulaRecord` writes `formula_source` / `formula_version_id` on a draft line and on a tier set.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.

mod pricing_formula_db;
mod rate_engine_common;

use pricing_formula_db::*;
use rate_engine_common::TestDb;
use sqlx::Row;
use uuid::Uuid;
use xc_rate::application::formula::formula_record::FormulaRecord;
use xc_rate::application::formula::formula_resolver::FormulaSource;

async fn draft_line(db: &TestDb) -> Uuid {
    let draft = Uuid::new_v4();
    sqlx::query("INSERT INTO rate_drafts (id, branch_id, status, created_by) VALUES ($1, 'MAIN', 'DRAFT', 'exec-1')")
        .bind(draft)
        .execute(&db.pool)
        .await
        .expect("a draft");
    let line = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rate_draft_lines (id, draft_id, branch_id, currency_code, buy_mode, sell_mode) \
         VALUES ($1, $2, 'MAIN', 'USD', 'AUTO', 'AUTO')",
    )
    .bind(line)
    .bind(draft)
    .execute(&db.pool)
    .await
    .expect("a draft line");
    line
}

async fn tier_set(db: &TestDb) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pricing_tier_sets (id, request_id, branch_id, currency_code, source, tiers, approved_by, approved_at) \
         VALUES ($1, $2, 'MAIN', 'USD', 'FOLLOW', '[]'::jsonb, 'exec-1', now())",
    )
    .bind(id)
    .bind(id.to_string())
    .execute(&db.pool)
    .await
    .expect("a tier set");
    id
}

async fn recorded(db: &TestDb, table: &str, id: Uuid) -> (Option<String>, Option<Uuid>) {
    let r = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT formula_source, formula_version_id FROM {table} WHERE id = $1"
    )))
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("the row");
    (r.get(0), r.get(1))
}

#[tokio::test]
async fn a_draft_line_records_each_source() {
    let db = TestDb::start().await;
    let v2 = org_formula(&db, 2, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    let pv = policy(&db, "MAIN", "0", "0", wide_guards()).await;
    for (source, want) in [
        (FormulaSource::Organisation(v2), (Some("ORGANISATION".to_owned()), Some(v2))),
        (FormulaSource::Branch(pv), (Some("BRANCH".to_owned()), None)),
        (FormulaSource::Default, (Some("DEFAULT".to_owned()), None)),
    ] {
        let line = draft_line(&db).await;
        let mut tx = db.begin().await;
        FormulaRecord::draft_line(&mut tx, line, source).await.expect("recorded");
        tx.commit().await.expect("commit");
        assert_eq!(recorded(&db, "rate_draft_lines", line).await, want, "{source:?}");
    }
}

#[tokio::test]
async fn a_tier_set_records_its_source() {
    let db = TestDb::start().await;
    let v2 = org_formula(&db, 2, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    let set = tier_set(&db).await;
    let mut tx = db.begin().await;
    FormulaRecord::tier_set(&mut tx, set, FormulaSource::Organisation(v2))
        .await
        .expect("recorded");
    tx.commit().await.expect("commit");
    assert_eq!(
        recorded(&db, "pricing_tier_sets", set).await,
        (Some("ORGANISATION".to_owned()), Some(v2))
    );
}
