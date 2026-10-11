//! The B04 transaction audit view (ticket #1294, design `desk-transaction-detail.md` §4.3).

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::transaction_audit::{
    TransactionAuditEventDto, TransactionAuditFilter, TransactionAuditViewData,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct TransactionAuditView {
    pool: PgPool,
}

impl BranchView for TransactionAuditView {
    const NAME: &'static str = "transaction_audit";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transactionView");
    const VERSIONED: bool = false;
    type Data = TransactionAuditViewData;
    type Filter = TransactionAuditFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let id_str = filter.id.clone();

        async move {
            let none = Ok(TransactionAuditViewData { events: None });
            let Some(id) = id_str.as_deref().and_then(|s| Uuid::from_str(s).ok()) else {
                return none;
            };

            let tx_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM transactions WHERE id = $1 AND branch_id = $2)",
            )
            .bind(id)
            .bind(branch.as_str())
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;

            if !tx_exists {
                return none;
            }

            let rows = sqlx::query(
                "SELECT at, actor_id, actor_role, action FROM audit_events \
                 WHERE object_ref = $1::text \
                    OR object_ref IN (SELECT id::text FROM reversals WHERE original_transaction_id = $1) \
                 ORDER BY at ASC, id ASC",
            )
            .bind(id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let events = rows
                .into_iter()
                .map(|r| {
                    let at: DateTime<Utc> = r.get("at");
                    TransactionAuditEventDto {
                        at: at.to_rfc3339(),
                        actor_id: r.get("actor_id"),
                        actor_role: r.get("actor_role"),
                        action: r.get("action"),
                    }
                })
                .collect();

            Ok(TransactionAuditViewData {
                events: Some(events),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(TransactionAuditView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::transaction_audit::export_types(dir)
}
