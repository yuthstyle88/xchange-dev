//! The `open_shift_choices` view (ticket CS-10 #6189).
//!
//! Answers what shifts a teller may open right now for a branch:
//! `{choices: [{templateId, name, businessDate, startsAt, endsAt, breakStartsAt?, breakEndsAt?}], preselect: templateId | null}`
//! from `choices_in(tx, branch, Utc::now())`.
//!
//! Inherits the policy row of `myShift` (`EntryPath::Rpc`, "myShift").

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::shift_plan::service::ShiftPlanService;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
pub use crate::presentation::dto::v3::open_shift_choices::{OpenShiftChoice, OpenShiftChoices};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

/// Computes the open shift choices at an arbitrary `now` instant for a branch.
pub async fn choices_at(
    pool: &PgPool,
    branch: &BranchId,
    now: DateTime<Utc>,
) -> AppResult<OpenShiftChoices> {
    let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
    let svc = ShiftPlanService::new(Arc::new(PgShiftTemplateStore), branch_tx);
    let mut tx = pool.begin().await.map_err(AppError::Database)?;
    let result = svc.choices_in(&mut tx, branch, now).await?;
    tx.commit().await.map_err(AppError::Database)?;

    let choices = result
        .choices
        .into_iter()
        .map(|c| {
            let (break_starts_at, break_ends_at) = match c.break_window {
                Some((s, e)) => (
                    Some(s.to_rfc3339_opts(SecondsFormat::Secs, true)),
                    Some(e.to_rfc3339_opts(SecondsFormat::Secs, true)),
                ),
                None => (None, None),
            };
            OpenShiftChoice {
                template_id: c.template_id,
                name: c.name,
                business_date: c.business_date.to_string(),
                starts_at: c.starts_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                ends_at: c.ends_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                break_starts_at,
                break_ends_at,
            }
        })
        .collect();

    Ok(OpenShiftChoices {
        choices,
        preselect: result.preselect,
    })
}

struct OpenShiftChoicesView {
    pool: PgPool,
}

impl BranchView for OpenShiftChoicesView {
    const NAME: &'static str = "open_shift_choices";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "myShift");
    type Data = OpenShiftChoices;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        async move { choices_at(&pool, &branch, Utc::now()).await }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(OpenShiftChoicesView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::open_shift_choices::export_types(dir)
}
