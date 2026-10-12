//! Export gate (EX-9 #1147; design `docs/design/blueprint/executive-finance.md` §8.4 D-8, §9 `ExportGate`; owner
//! decision E-2: only a holder of `ReportsExport`, i.e. EXECUTIVE and FINANCE, may export).
//!
//! A paged view read with `purpose=export` is an export. The view core (`view.rs`, `handle_paged_view`) asks
//! [`ExportGate::check`] right after the view's own `Policy::authorize`, against the branch that check resolved
//! (the lane controller's ruling on #1147, issuecomment-6100577886: a FINANCE at B1 who is TELLER at B3 cannot export
//! B3), and before any row is read. When rows are answered, [`ExportGate::record`] appends one `audit_events` row per
//! request through the shared `AuditLog`; if that fails, no rows leave the server.

use actix_web::http::StatusCode;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::access::authorizer::{Authorizer, Target};
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

use super::error::V3Error;

/// Who may export at which branch, and through which role: what the audit row records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportGrant {
    pub actor_id: String,
    /// The first role of the `ReportsExport` grant, e.g. `FINANCE`.
    pub role: String,
    pub branch: BranchId,
}

/// The one export rule for every paged view (♻️: no per-view copy).
pub struct ExportGate;

impl ExportGate {
    /// `ReportsExport` at `branch` (the branch the view resolved), or `outOfScope` for any refusal.
    pub fn check(
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> Result<ExportGrant, V3Error> {
        let target = Target::new(&actor.organization_id, Some(branch.clone()), None);
        let refused = || V3Error::from(PolicyError::OutOfScope);
        let grant = Authorizer::check(Some(actor), Permission::ReportsExport, &target, now)
            .map_err(|_| refused())?;
        let role = grant.roles_used.first().ok_or_else(refused)?;
        Ok(ExportGrant {
            actor_id: actor.user_id.clone(),
            role: role.as_str().to_owned(),
            branch: branch.clone(),
        })
    }

    /// One `audit_events` row for an export of `rows` rows of `view` (design §8.4): `after_redacted` is
    /// `{"filter": <the query's filter keys>, "rows": <count>}`, through the shared `Redactor`; never row content.
    /// In its own transaction. Any failure is `500 serverError`, so the caller answers no rows (fail closed).
    pub async fn record(
        pool: &PgPool,
        grant: &ExportGrant,
        view: &'static str,
        filter: Value,
        rows: usize,
    ) -> Result<(), V3Error> {
        let event = AuditEvent {
            actor_id: grant.actor_id.clone(),
            actor_role: grant.role.clone(),
            scope: "BRANCH".to_owned(),
            action: "EXPORT".to_owned(),
            object_ref: format!("view:{view}"),
            branch_id: Some(grant.branch.as_str().to_owned()),
            before: None,
            after: Some(json!({ "filter": filter, "rows": rows })),
        };
        let failed = |e: &dyn std::fmt::Display| {
            tracing::error!(error = %e, view, "export audit failed");
            V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        };
        let mut tx = pool.begin().await.map_err(|e| failed(&e))?;
        PgAuditLog::new(pool.clone())
            .append(&mut tx, &event)
            .await
            .map_err(|e| failed(&e))?;
        tx.commit().await.map_err(|e| failed(&e))?;
        Ok(())
    }
}
