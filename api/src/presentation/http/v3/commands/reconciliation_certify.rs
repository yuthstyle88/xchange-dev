//! FN-1 #1068: `POST /api/v3/commands/reconciliation_certify` `{branch, date, figuresHash, note}` (design
//! `executive-finance.md` §8.2; owner decision E-8). FINANCE certifies a branch-day's sheet as it read it, in the
//! seam's one transaction under the branch lock: the sheet is recomputed (another hash ⇒ `409 staleVersion`), the
//! verifier of one of that day's counts is refused (`409 samePerson`, `CashError::SameCounter`'s sentence), then one
//! `reconciliation_certifications` row and one `audit_events` row are written. The seam replays a repeated key and
//! refuses the same key with another body (`idempotencyConflict`).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::domain::error::CashError;
use xc_kernel::ids::BranchId;

use crate::application::finance::reconciliation::{
    CertifyRefusal, ReconciliationService, business_date,
};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::finance_reconciliation::{
    CertificationRow, ReconciliationCertifyInput,
};
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::me::me_roles;

pub const AUDIT_ACTION: &str = "RECONCILIATION_CERTIFY";

pub struct ReconciliationCertifyCommand {
    pool: PgPool,
    service: ReconciliationService,
}

impl ReconciliationCertifyCommand {
    pub fn new(pool: PgPool) -> Self {
        Self {
            service: ReconciliationService::standard(pool.clone()),
            pool,
        }
    }
}

impl CommandAction for ReconciliationCertifyCommand {
    const NAME: &'static str = "reconciliationCertify";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "reconciliationCertify");
    type Input = ReconciliationCertifyInput;
    type Output = CertificationRow;
    const ROUTE: &'static str = "/api/v3/commands/reconciliation_certify";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let Some(date) = business_date(&input.date) else {
            return Err(V3Error::new(
                "invalidRequest",
                Some("date"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        };
        let certifier = Uuid::parse_str(&who.user_id).map_err(|_| server_error())?;
        let note = (!input.note.is_empty()).then_some(input.note.as_str());
        let answer = self
            .service
            .certify_in(
                tx,
                branch,
                date,
                certifier,
                &input.figures_hash,
                note,
                Utc::now(),
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "reconciliation certify failed");
                server_error()
            })?;
        let row = match answer {
            Ok(row) => row,
            Err(CertifyRefusal::StaleVersion) => {
                return Err(V3Error::new(
                    "staleVersion",
                    None,
                    "ตัวเลขเปลี่ยนไปแล้ว กรุณาโหลดใหม่และตรวจสอบอีกครั้ง",
                    StatusCode::CONFLICT,
                ));
            }
            Err(CertifyRefusal::SamePerson) => {
                return Err(V3Error::new(
                    "samePerson",
                    None,
                    CashError::SameCounter.to_string(),
                    StatusCode::CONFLICT,
                ));
            }
        };

        // The audit row records the caller's valid-now Blueprint roles (design §3.3), as EX-1 does.
        let mut roles: Vec<String> = me_roles(who, Utc::now())
            .into_iter()
            .map(|r| r.role)
            .collect();
        roles.dedup();
        PgAuditLog::new(self.pool.clone())
            .append(
                tx,
                &AuditEvent {
                    actor_id: who.user_id.clone(),
                    actor_role: roles.join(","),
                    scope: "branch".to_owned(),
                    action: AUDIT_ACTION.to_owned(),
                    object_ref: format!("reconciliation:{}:{}", branch.as_str(), input.date),
                    branch_id: Some(branch.as_str().to_owned()),
                    before: None,
                    after: Some(json!({ "figuresHash": input.figures_hash, "note": input.note })),
                },
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "reconciliation certify audit failed");
                server_error()
            })?;
        Ok(row)
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(ReconciliationCertifyCommand::new(state.db.clone())),
    );
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    // the input and the answer are exported with the view's types, in `finance_reconciliation.ts`
    Ok(())
}
