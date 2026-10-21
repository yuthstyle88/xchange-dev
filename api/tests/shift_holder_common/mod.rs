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
/// test user by `name` in the XC organisation, seeded if missing. Not for `"owner"`: the suites' owner user already
/// exists and is found by the core.
pub async fn user_of(pool: &PgPool, saver: &str) -> String {
    if let Some(id) =
        sqlx::query_scalar::<_, String>("SELECT id::text FROM users WHERE id::text = $1")
            .bind(saver)
            .fetch_optional(pool)
            .await
            .expect("look up the saver as a users.id")
    {
        return id;
    }
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT u.id::text FROM users u JOIN organizations o ON o.id = u.organization_id \
         WHERE o.code = 'XC' AND u.name = $1 AND u.created_by = 'test' \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .bind(saver)
    .fetch_optional(pool)
    .await
    .expect("look up the saver's user by name")
    {
        return id;
    }
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT id, $1, 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
         RETURNING id::text",
    )
    .bind(saver)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("the user of saver {saver}: {e}"))
}

/// An OPEN shift of `saver` at `branch`, written straight into `shifts` (no `opened_by_count`), with `opening`
/// posted into its ledger drawer from `Bank` as `Opening`. Returns the shift id.
pub async fn open_shift_for(
    pool: &PgPool,
    branch: &str,
    saver: &str,
    opening: &[(&str, &str)],
) -> String {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let teller = user_of(pool, saver).await;
    if let Some(existing) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM shifts WHERE branch_id = $1 AND teller_id = $2::uuid AND status = 'OPEN' LIMIT 1",
    )
    .bind(branch)
    .bind(&teller)
    .fetch_optional(pool)
    .await
    .expect("check the saver's existing OPEN shift at branch")
    {
        return existing;
    }
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
    let mut tx = pool.begin().await.expect("begin");
    for (code, amount) in opening {
        let quantity: rust_decimal::Decimal = amount.parse().expect("an opening amount");
        if quantity <= rust_decimal::Decimal::ZERO {
            continue;
        }
        xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Drawer(shift.clone()),
                    currency: xc_kernel::money::CurrencyCode::parse(code)
                        .expect("a register currency"),
                    quantity: xc_kernel::money::ForeignAmount::from_decimal(quantity),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: format!("r1-fund:{shift}:{code}"),
                        source_key: "fund".to_string(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("fund the open shift's drawer through the ledger");
    }
    tx.commit().await.expect("commit");
    shift
}

/// The branch day of `branch` OPEN, as `dayStartService` leaves it (`ops_state.phase`), written straight into the
/// table. Since ruling A on #626 a compat save meets the teller gate (`check_teller_save_in`), which refuses a day
/// that is not OPEN or CUT_OFF (`SAVE_CLOSED`); a verified OPEN count only moves the day to READY. Setup only, the
/// same seed the passing suites write (`ca10p_desk_draft_begin_save_test.rs`, `v3_staff_views_test.rs`).
pub async fn open_day(pool: &PgPool, branch: &str) {
    sqlx::query(
        "INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN') \
         ON CONFLICT (branch_id) DO UPDATE SET phase = 'OPEN'",
    )
    .bind(branch)
    .execute(pool)
    .await
    .expect("open the branch day");
}
