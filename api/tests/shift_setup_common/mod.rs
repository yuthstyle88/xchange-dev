//! Shared Blueprint cash count helper for test setup (CA-10a #1632).
//! Replaces legacy v2 direct shift actions (adminOpenShift / adminCloseShift)
//! with CashCountService submit + verify by a different user.

#![allow(dead_code)]

#[allow(clippy::duplicate_mod)]
#[path = "../lg4_fund/mod.rs"]
mod lg4_fund;

use std::str::FromStr;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::{Actor as AuthActor, Role as AuthRole, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_kernel::ids::BranchId;

/// Builds CountLines from a JSON object mapping currency -> amount (number or string).
/// Uses existing denominations where possible, and provisions denomination records
/// where needed so that the submitted count exactly matches the requested amount.
async fn lines_from_values(pool: &PgPool, amounts: &Value) -> Vec<CountLine> {
    let mut lines = Vec::new();
    if let Some(obj) = amounts.as_object() {
        for (curr, amt_val) in obj {
            let mut remaining = match amt_val {
                Value::String(s) => Decimal::from_str(s).expect("valid decimal string"),
                Value::Number(n) => {
                    Decimal::from_str(&n.to_string()).expect("valid decimal number")
                }
                _ => panic!("unexpected counted value type for {curr}: {amt_val:?}"),
            };

            if remaining < Decimal::ZERO {
                panic!("count amount for {curr} cannot be negative: {remaining}");
            }

            let denoms: Vec<(i64, Decimal)> = sqlx::query_as(
                "SELECT id, face_value FROM denominations WHERE currency_code = $1 AND face_value IS NOT NULL AND face_value > 0 ORDER BY face_value DESC",
            )
            .bind(curr)
            .fetch_all(pool)
            .await
            .expect("query denominations");

            for (denom_id, face) in denoms {
                if remaining >= face {
                    let qty = (remaining / face).floor();
                    let qty_i32 = qty.to_i32().expect("qty fits i32");
                    if qty_i32 > 0 {
                        lines.push(CountLine::new(curr, denom_id, qty_i32));
                        remaining -= face * qty;
                    }
                }
            }

            if remaining > Decimal::ZERO {
                // If remaining is a whole number, use face_value = 1
                let (face_val, qty) = if remaining.fract().is_zero() {
                    (
                        Decimal::ONE,
                        remaining
                            .to_i32()
                            .expect("amount as integer quantity with face_value 1"),
                    )
                } else {
                    // Fractional amount: provision denomination with face_value = remaining
                    (remaining, 1)
                };

                let denom_id: i64 = match sqlx::query_scalar(
                    "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2",
                )
                .bind(curr)
                .bind(face_val)
                .fetch_optional(pool)
                .await
                .expect("query denomination")
                {
                    Some(id) => id,
                    None => {
                        sqlx::query_scalar(
                            "INSERT INTO denominations (currency_code, face_value, group_code) VALUES ($1, $2, 'ALL') RETURNING id",
                        )
                        .bind(curr)
                        .bind(face_val)
                        .fetch_one(pool)
                        .await
                        .expect("insert denomination")
                    }
                };

                lines.push(CountLine::new(curr, denom_id, qty));
            }
        }
    }
    lines
}

/// Resolves or provisions a teller user and a verifier user (who must be distinct)
/// for `branch`, and returns their respective AuthActors and the branch's organization_id.
async fn setup_actors(
    pool: &PgPool,
    branch: &str,
    preferred_teller: Option<Uuid>,
) -> (AuthActor, AuthActor, Uuid) {
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
        .bind(branch)
        .fetch_one(pool)
        .await
        .expect("branch organization_id");

    let teller_user_id: Uuid = if let Some(t_id) = preferred_teller {
        t_id
    } else {
        match sqlx::query_scalar(
            "SELECT u.id FROM users u \
             LEFT JOIN employees e ON e.id = u.legacy_employee_id \
             WHERE (e.role IN ('FRONT', 'TELLER') OR u.legacy_employee_id IS NOT NULL) \
               AND NOT EXISTS (SELECT 1 FROM shifts s WHERE s.teller_id = u.id AND s.status = 'OPEN') \
             ORDER BY (e.branch_id = $1) DESC, (e.role IN ('FRONT', 'TELLER')) DESC \
             LIMIT 1",
        )
        .bind(branch)
        .fetch_optional(pool)
        .await
        .expect("query teller user")
        {
            Some(id) => id,
            None => {
                sqlx::query_scalar(
                    "INSERT INTO users (id, organization_id, name, status, created_by) \
                     VALUES (gen_random_uuid(), $1, 'shift_setup_teller', 'ACTIVE', 'test') \
                     RETURNING id",
                )
                .bind(org_id)
                .fetch_one(pool)
                .await
                .expect("create teller user")
            }
        }
    };

    let verifier_user_id: Uuid = match sqlx::query_scalar(
        "SELECT u.id FROM users u \
         LEFT JOIN employees e ON e.id = u.legacy_employee_id \
         WHERE u.id != $1 \
           AND (e.role = 'MANAGER' OR e.id IS NOT NULL) \
         ORDER BY (e.role = 'MANAGER') DESC, (e.branch_id = $2) DESC \
         LIMIT 1",
    )
    .bind(teller_user_id)
    .bind(branch)
    .fetch_optional(pool)
    .await
    .expect("query verifier user")
    {
        Some(id) => id,
        None => sqlx::query_scalar(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
                 VALUES (gen_random_uuid(), $1, 'shift_setup_verifier', 'ACTIVE', 'test') \
                 RETURNING id",
        )
        .bind(org_id)
        .fetch_one(pool)
        .await
        .expect("create verifier user"),
    };

    let teller_actor = AuthActor::new(
        &teller_user_id.to_string(),
        &org_id.to_string(),
        vec![ScopedRole::new(
            AuthRole::Teller,
            Some(BranchId::new(branch)),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    );

    let verifier_actor = AuthActor::new(
        &verifier_user_id.to_string(),
        &org_id.to_string(),
        vec![ScopedRole::new(
            AuthRole::BranchManager,
            Some(BranchId::new(branch)),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    );

    (teller_actor, verifier_actor, org_id)
}

/// Opens a shift in `branch` by count via CashCountService:
/// Funds the SAFE, submits the OPEN count by a TELLER, and verifies it by a different user (MANAGER).
/// Returns the new open shift's ID.
pub async fn open_shift_by_count(
    pool: &PgPool,
    branch: &str,
    opening: Value,
    now: DateTime<Utc>,
) -> String {
    lg4_fund::lg4_fund_safe(pool, branch).await;

    let (teller_actor, verifier_actor, org_id) = setup_actors(pool, branch, None).await;

    sqlx::query(
        "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
         VALUES ($1, $2, 1000000, 'test', now()) \
         ON CONFLICT (organization_id, branch_id) DO UPDATE SET count_difference_limit_thb = 1000000",
    )
    .bind(org_id)
    .bind(branch)
    .execute(pool)
    .await
    .expect("set cash limits");

    let lines = lines_from_values(pool, &opening).await;
    let cash_count_service = CashCountService::pg(pool.clone());

    let count = cash_count_service
        .submit(
            &teller_actor,
            SubmitCount {
                branch: BranchId::new(branch),
                shift_id: None,
                kind: CountKind::Open,
                lines,
                note: None,
                idempotency_key: format!("open-count-{}", Uuid::new_v4()),
            },
            now,
        )
        .await
        .expect("teller submits OPEN count");

    let verified = cash_count_service
        .verify(&verifier_actor, &count.id, now)
        .await
        .expect("manager verifies OPEN count");

    // Maintain branch_versions so any view watching `since` observes the open event
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version, changed_at) VALUES ($1, 1, $2) \
         ON CONFLICT (branch_id) DO UPDATE SET version = branch_versions.version + 1, changed_at = $2",
    )
    .bind(branch)
    .bind(now)
    .execute(pool)
    .await
    .expect("bump branch version");

    let shift_id = verified
        .shift_id
        .expect("a verified OPEN count names its shift");

    shift_id
}

/// Closes the open shift in `branch` by count via CashCountService:
/// The shift's teller submits the CLOSE count, and a different user (MANAGER) verifies it.
/// A note is attached to the count. Returns the closed shift's ID.
pub async fn close_shift_by_count_with_note(
    pool: &PgPool,
    branch: &str,
    counted: Value,
    note: &str,
    now: DateTime<Utc>,
) -> String {
    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch)
            .fetch_one(pool)
            .await
            .expect("an open shift to close");

    let shift_teller: Option<Uuid> =
        sqlx::query_scalar("SELECT teller_id FROM shifts WHERE id = $1")
            .bind(&shift_id)
            .fetch_one(pool)
            .await
            .expect("fetch shift teller_id");

    let (teller_actor, verifier_actor, org_id) = setup_actors(pool, branch, shift_teller).await;

    if shift_teller.is_none() {
        sqlx::query("UPDATE shifts SET teller_id = $1::uuid WHERE id = $2")
            .bind(&teller_actor.user_id)
            .bind(&shift_id)
            .execute(pool)
            .await
            .expect("update shift teller_id");
    }

    sqlx::query(
        "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
         VALUES ($1, $2, 1000000, 'test', now()) \
         ON CONFLICT (organization_id, branch_id) DO UPDATE SET count_difference_limit_thb = 1000000",
    )
    .bind(org_id)
    .bind(branch)
    .execute(pool)
    .await
    .expect("set cash limits");

    let lines = lines_from_values(pool, &counted).await;
    let cash_count_service = CashCountService::pg(pool.clone());

    let count = cash_count_service
        .submit(
            &teller_actor,
            SubmitCount {
                branch: BranchId::new(branch),
                shift_id: Some(shift_id.clone()),
                kind: CountKind::Close,
                lines,
                note: Some(note.to_string()),
                idempotency_key: format!("close-count-{}", Uuid::new_v4()),
            },
            now,
        )
        .await
        .expect("teller submits CLOSE count");

    cash_count_service
        .verify(&verifier_actor, &count.id, now)
        .await
        .expect("manager verifies CLOSE count");

    // Maintain branch_versions so any view watching `since` observes the close event
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version, changed_at) VALUES ($1, 1, $2) \
         ON CONFLICT (branch_id) DO UPDATE SET version = branch_versions.version + 1, changed_at = $2",
    )
    .bind(branch)
    .bind(now)
    .execute(pool)
    .await
    .expect("bump branch version");

    shift_id
}

/// Closes the open shift in `branch` by count via CashCountService with default note.
pub async fn close_shift_by_count(
    pool: &PgPool,
    branch: &str,
    counted: Value,
    now: DateTime<Utc>,
) -> String {
    close_shift_by_count_with_note(pool, branch, counted, "close by count", now).await
}
