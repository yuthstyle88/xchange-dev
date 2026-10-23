//! #3413 (section B S1): `POST /api/v3/commands/cash_exception_raise` appends an `OPEN`
//! manual `cash_exceptions` row on the branch with a reason and signed currency differences.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Refusal, Target};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, QuantityDelta};
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
use xc_shift::infrastructure::custody::custody_repository::append_cash_exception_in;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::cash_exception::{
    CashExceptionDifferenceInput, CashExceptionRaiseInput, CashExceptionRaiseOutput,
};
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

fn audit_actor_role(who: &Actor) -> String {
    who.roles
        .first()
        .map(|s| s.role.as_str())
        .unwrap_or("STAFF")
        .to_string()
}

pub struct CashExceptionRaiseCommand {
    pool: PgPool,
}

impl CashExceptionRaiseCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn parse_differences(items: &[CashExceptionDifferenceInput]) -> Result<StockDelta, V3Error> {
    if items.is_empty() {
        return Err(V3Error::new(
            "validationFailed",
            Some("differences"),
            "กรุณาระบุผลต่างอย่างน้อยหนึ่งสกุลเงิน",
            StatusCode::BAD_REQUEST,
        ));
    }

    let mut seen = HashSet::with_capacity(items.len());
    let mut pairs = Vec::with_capacity(items.len());

    for item in items {
        let code = CurrencyCode::parse(item.currency.trim()).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("differences"),
                "รหัสสกุลเงินไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
        })?;

        if !seen.insert(code) {
            return Err(V3Error::new(
                "validationFailed",
                Some("differences"),
                "สกุลเงินซ้ำกัน",
                StatusCode::BAD_REQUEST,
            ));
        }

        let raw_amount = item.amount.trim();
        let (negative, unsigned) = match raw_amount.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, raw_amount),
        };

        let money = Money::parse(unsigned).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("differences"),
                "จำนวนเงินไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
        })?;

        if money.is_zero() {
            return Err(V3Error::new(
                "validationFailed",
                Some("differences"),
                "จำนวนผลต่างต้องไม่เป็นศูนย์",
                StatusCode::BAD_REQUEST,
            ));
        }

        let amt = ForeignAmount::checked_from_decimal(rust_decimal::Decimal::from_i128_with_scale(
            money.minor_i128(),
            2,
        ))
        .expect("Money fits in ForeignAmount");
        let delta = if negative {
            QuantityDelta::ZERO.checked_minus(amt).unwrap()
        } else {
            QuantityDelta::from(amt)
        };
        pairs.push((code, delta));
    }

    Ok(StockDelta::of(&pairs))
}

impl CommandAction for CashExceptionRaiseCommand {
    const NAME: &'static str = "cashExceptionRaise";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Input = CashExceptionRaiseInput;
    type Output = CashExceptionRaiseOutput;
    const ROUTE: &'static str = "/api/v3/commands/cash_exception_raise";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let now = Utc::now();
        let target = Target::new(
            &who.organization_id,
            Some(branch.clone()),
            who.customer_id.as_deref(),
        );
        if let Err(refusal) =
            Authorizer::check(Some(who), Permission::FinanceReconcile, &target, now)
        {
            return Err(match refusal {
                Refusal::OutOfScope => out_of_scope(),
                _ => V3Error::from(
                    xc_auth::domain::policy::value_objects::policy_error::PolicyError::RoleNotAllowedHere,
                ),
            });
        }

        let reason = input.reason.trim();
        if reason.is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("reason"),
                "กรุณาระบุสาเหตุ",
                StatusCode::BAD_REQUEST,
            ));
        }

        let place = input.place.trim();
        if place != "DRAWER" && place != "SAFE" {
            return Err(V3Error::new(
                "validationFailed",
                Some("place"),
                "สถานที่ต้องเป็น DRAWER หรือ SAFE",
                StatusCode::BAD_REQUEST,
            ));
        }

        let delta = parse_differences(&input.differences)?;
        let (cash_variance, safe_variance) = if place == "DRAWER" {
            (Some(delta), None)
        } else {
            (None, Some(delta))
        };

        let exception_id = format!("EX-{}", Uuid::new_v4());
        let exception = CashException {
            id: exception_id.clone(),
            request_id: format!("MANUAL-{}", Uuid::new_v4()),
            status: ExceptionStatus::Open,
            target_difference: None,
            cash_variance,
            safe_variance,
            note: format!("MANUAL {reason}"),
            at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
            reviewed_by: None,
            reviewed_at: None,
        };

        append_cash_exception_in(tx, branch, &exception)
            .await
            .map_err(|_| server_error())?;

        PgAuditLog::new(self.pool.clone())
            .append(
                tx,
                &AuditEvent {
                    actor_id: who.user_id.clone(),
                    actor_role: audit_actor_role(who),
                    scope: format!("BRANCH:{}", branch.as_str()),
                    action: "CASH_EXCEPTION_RAISE".to_owned(),
                    object_ref: format!("cash_exception:{exception_id}"),
                    branch_id: Some(branch.as_str().to_owned()),
                    before: None,
                    after: Some(json!({
                        "status": "OPEN",
                        "place": place,
                        "reason": reason,
                    })),
                },
            )
            .await
            .map_err(|_| server_error())?;

        Ok(CashExceptionRaiseOutput {
            exception_id,
            status: "OPEN".to_owned(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(CashExceptionRaiseCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CashExceptionDifferenceInput::export_all(&cfg)?;
    CashExceptionRaiseInput::export_all(&cfg)?;
    CashExceptionRaiseOutput::export_all(&cfg)?;
    Ok(())
}
