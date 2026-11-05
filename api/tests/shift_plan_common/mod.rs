//! CS-4s setup tickets (docs/design/configurable-shifts.md §2.7 "Fixtures"): after CS-4 the custody window comes from the
//! branch's shift plan, not from `opening_hours`, so a suite that shaped the window with `opening_hours` seeds a plan
//! instead. Written by the sub-controller claude/35271802; do not edit. A suite that uses it declares
//! `mod shift_plan_common;` at its root.
//!
//! Every new branch already gets the defaults A 06–14, B 14–22, C 22–06 from the `branches` trigger (owner Q10 = A).
#![allow(dead_code)]

use sqlx::PgPool;

/// Retires `branch`'s live plan and stores `rows` of `(name, starts_at, ends_at)` as `HH:MM`, in that order. Stored by
/// SQL, so it is not validated: `&[]` leaves the branch with no live template.
pub async fn set_plan(pool: &PgPool, branch: &str, rows: &[(&str, &str, &str)]) {
    sqlx::query(
        "UPDATE shift_templates SET retired_at = now() WHERE branch_id = $1 AND retired_at IS NULL",
    )
    .bind(branch)
    .execute(pool)
    .await
    .expect("retire the branch's plan");
    for (i, (name, start, end)) in rows.iter().enumerate() {
        sqlx::query(
            "INSERT INTO shift_templates (organization_id, branch_id, name, starts_at, ends_at, position) \
             SELECT organization_id, id, $2, $3::time, $4::time, $5 FROM branches WHERE id = $1",
        )
        .bind(branch)
        .bind(name)
        .bind(start)
        .bind(end)
        .bind(i as i16)
        .execute(pool)
        .await
        .expect("insert a template");
    }
}

/// Leaves `branch` with no live template, so its custody window is `NotSet`.
pub async fn clear_plan(pool: &PgPool, branch: &str) {
    set_plan(pool, branch, &[]).await;
}

/// Sets `branch`'s timezone, which the plan's business day is read in.
pub async fn set_timezone(pool: &PgPool, branch: &str, timezone: &str) {
    sqlx::query("UPDATE branches SET timezone = $2 WHERE id = $1")
        .bind(branch)
        .bind(timezone)
        .execute(pool)
        .await
        .expect("set the branch's timezone");
}
