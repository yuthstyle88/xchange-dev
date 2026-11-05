#![allow(dead_code)]

use sqlx::PgPool;

/// Dates every OPEN shift of `branch` to `date` (YYYY-MM-DD), so a LIVE save made next stores `date` as its
/// value_date (CS-8, design §2.4). Fixtures only.
pub async fn date_open_shifts(pool: &PgPool, branch: &str, date: &str) {
    sqlx::query(
        "UPDATE shifts SET business_date = $2::date WHERE branch_id = $1 AND status = 'OPEN'",
    )
    .bind(branch)
    .bind(date)
    .execute(pool)
    .await
    .expect("date the open shift");
}
