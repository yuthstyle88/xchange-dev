//! LG-3b (#926): `PUT /api/cash-limits` saves a branch's count-difference limit.
//!
//! Upserts the branch's `cash_limits` row (ledger design §3.3) and answers it in the `cash_limits`
//! view's shape. `null` stores NULL: no limit, so every non-zero difference goes to FINANCE.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Bound, Sign};

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::cash_limits::{CashLimitsData, CashLimitsSaveInput};
use crate::presentation::http::v3;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

pub struct CashLimitsSaveCommand;

/// A limit in baht: a decimal string, zero or more, with at most 18 integer digits and at most
/// two decimal places (the column is `numeric(20,2)` with `CHECK (>= 0)`).
fn parse_limit(text: &str) -> Result<Decimal, V3Error> {
    v3::input::decimal(
        "countDifferenceLimitThb",
        text,
        Bound::numeric(20, 2, Sign::NonNegative),
    )
}

impl CommandAction for CashLimitsSaveCommand {
    const NAME: &'static str = "cashLimitsSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "cashLimitsSave");
    type Input = CashLimitsSaveInput;
    type Output = CashLimitsData;
    const ROUTE: &'static str = "/api/cash-limits";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let limit = input
            .count_difference_limit_thb
            .as_deref()
            .map(parse_limit)
            .transpose()?;

        let row: (Option<String>, String, DateTime<Utc>) = sqlx::query_as(
            "INSERT INTO cash_limits \
                 (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             SELECT b.organization_id, b.id, $2::numeric, $3, now() \
               FROM branches b WHERE b.id = $1 \
             ON CONFLICT (organization_id, branch_id) DO UPDATE \
                SET count_difference_limit_thb = EXCLUDED.count_difference_limit_thb, \
                    updated_by = EXCLUDED.updated_by, \
                    updated_at = EXCLUDED.updated_at \
             RETURNING count_difference_limit_thb::text, updated_by, updated_at",
        )
        .bind(branch.as_str())
        .bind(limit.map(|l| l.to_string()))
        .bind(&who.user_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        Ok(CashLimitsData::of(branch.as_str(), Some(row)))
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(CashLimitsSaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CashLimitsSaveInput::export_all(&cfg)?;
    CashLimitsData::export_all(&cfg)?;
    Ok(())
}
