//! CA-10 (#626) setup tickets: who holds a branch's OPEN shift, and an OPEN shift for a given saver.
//!
//! After CA-10 a save moves stock on the SAVER's own OPEN shift (`ShiftService::teller_of_actor_in`, #3017) and,
//! after M3, one teller holds at most one OPEN shift. A suite that opens a shift for one person and saves as another
//! must save as the shift's holder (and upload its files as the same id), or open a shift for its saver.
//!
//! Provided by the ticket author (claude/dell-claude-1) for the #626 setup tickets; do not edit. A suite that uses it
//! declares `mod shift_holder_common;` at its root.
#![allow(dead_code)]

use sqlx::PgPool;

/// The `users.id` of the teller holding `branch`'s newest OPEN shift. Pass it as the saver (it resolves to itself)
/// and as the uploader of the save's files.
pub async fn teller_of_open_shift(pool: &PgPool, branch: &str) -> String {
    sqlx::query_scalar(
        "SELECT teller_id::text FROM shifts WHERE branch_id = $1 AND status = 'OPEN' \
         ORDER BY opened_at DESC LIMIT 1",
    )
    .bind(branch)
    .fetch_optional(pool)
    .await
    .expect("read the branch's OPEN shift")
    .unwrap_or_else(|| panic!("no OPEN shift at {branch}"))
}

/// The `users.id` a saver id answers to, as `teller_of_actor_in` resolves the ids the suites use: a `users.id`, or a
/// `users.legacy_employee_id` (an employee id), seeded in the XC organisation if missing. Not for `"owner"`: the
/// suites' owner user already exists and is found by the core.
pub async fn user_of(pool: &PgPool, saver: &str) -> String {
    if let Some(id) = sqlx::query_scalar::<_, String>("SELECT id::text FROM users WHERE id::text = $1")
        .bind(saver)
        .fetch_optional(pool)
        .await
        .expect("look up the saver as a users.id")
    {
        return id;
    }
    sqlx::query(
        "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
         SELECT id, $1, 'PENDING', 'test', $1 FROM organizations WHERE code = 'XC' \
         ON CONFLICT (legacy_employee_id) DO NOTHING",
    )
    .bind(saver)
    .execute(pool)
    .await
    .expect("seed the saver's user");
    sqlx::query_scalar("SELECT id::text FROM users WHERE legacy_employee_id = $1")
        .bind(saver)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("the user of saver {saver}: {e}"))
}

/// An OPEN shift of `saver` at `branch`, written straight into the tables (no `opened_by_count`), with `opening` as
/// its opening and expected balances. Returns the shift id. Fund its ledger drawer yourself where the suite's
/// postings need it (e.g. `save_common::fund_ledger_drawer`).
pub async fn open_shift_for(pool: &PgPool, branch: &str, saver: &str, opening: &[(&str, &str)]) -> String {
    let teller = user_of(pool, saver).await;
    let shift = format!("SH-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now() - interval '1 hour', $3, '', $4, $5::uuid)",
    )
    .bind(&shift)
    .bind(branch)
    .bind(saver)
    .bind(format!("open-{}", uuid::Uuid::new_v4()))
    .bind(&teller)
    .execute(pool)
    .await
    .expect("seed the saver's OPEN shift");
    for (code, amount) in opening {
        sqlx::query(
            "INSERT INTO shift_balances (shift_id, code, opening, expected) VALUES ($1, $2, $3::numeric, $3::numeric)",
        )
        .bind(&shift)
        .bind(code)
        .bind(amount)
        .execute(pool)
        .await
        .expect("seed an opening balance");
    }
    shift
}
