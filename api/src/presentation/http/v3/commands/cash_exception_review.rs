//! #3413 (section B S1): `POST /api/v3/commands/cash_exception_review` marks an open
//! `cash_exceptions` row of the branch as `REVIEWED` with a non-empty review note.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use ts_rs::TS;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Refusal, Target};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_shift::infrastructure::custody::exception_review::{ReviewOutcome, review_in};

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::cash_exception::{
    CashExceptionReviewInput, CashExceptionReviewOutput,
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

pub struct CashExceptionReviewCommand {
    pool: PgPool,
}

impl CashExceptionReviewCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for CashExceptionReviewCommand {
    const NAME: &'static str = "cashExceptionReview";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Input = CashExceptionReviewInput;
    type Output = CashExceptionReviewOutput;
    const ROUTE: &'static str = "/api/v3/commands/cash_exception_review";
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

        let exception_id = input.exception_id.trim();
        if exception_id.is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("exceptionId"),
                "กรุณาระบุรายการผิดปกติ",
                StatusCode::BAD_REQUEST,
            ));
        }

        let note = input.note.trim();
        if note.is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("note"),
                "กรุณาระบุหมายเหตุการตรวจสอบ",
                StatusCode::BAD_REQUEST,
            ));
        }

        let outcome = review_in(tx, branch, exception_id, &who.user_id, note, now)
            .await
            .map_err(|_| server_error())?;

        match outcome {
            ReviewOutcome::NotFound => {
                return Err(V3Error::new(
                    "notFound",
                    Some("exceptionId"),
                    "ไม่พบรายการผิดปกติ",
                    StatusCode::NOT_FOUND,
                ));
            }
            ReviewOutcome::AlreadyReviewed => {
                return Err(V3Error::new(
                    "alreadyReviewed",
                    Some("exceptionId"),
                    "รายการนี้ได้รับการตรวจสอบแล้ว",
                    StatusCode::CONFLICT,
                ));
            }
            ReviewOutcome::Reviewed => {}
        }

        PgAuditLog::new(self.pool.clone())
            .append(
                tx,
                &AuditEvent {
                    actor_id: who.user_id.clone(),
                    actor_role: audit_actor_role(who),
                    scope: format!("BRANCH:{}", branch.as_str()),
                    action: "CASH_EXCEPTION_REVIEW".to_owned(),
                    object_ref: format!("cash_exception:{exception_id}"),
                    branch_id: Some(branch.as_str().to_owned()),
                    before: Some(json!({ "status": "OPEN" })),
                    after: Some(json!({
                        "status": "REVIEWED",
                        "reviewedBy": who.user_id,
                        "reviewNote": note,
                    })),
                },
            )
            .await
            .map_err(|_| server_error())?;

        Ok(CashExceptionReviewOutput {
            exception_id: exception_id.to_owned(),
            status: "REVIEWED".to_owned(),
            reviewed_by: who.user_id.clone(),
            reviewed_at: now.to_rfc3339(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(CashExceptionReviewCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CashExceptionReviewInput::export_all(&cfg)?;
    CashExceptionReviewOutput::export_all(&cfg)?;
    Ok(())
}
