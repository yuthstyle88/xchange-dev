//! Admin `admin_booking_slots` view (BK-6, #1388).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_booking_slots::{
    AdminBookingPolicyDto, AdminBookingSlotsData, AdminBookingSlotsFilter, AdminClosureDto,
    AdminSlotDto, AdminSlotRuleDto,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct AdminBookingSlotsView {
    pool: PgPool,
}

impl AdminBookingSlotsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type RuleDbRow = (String, i32, i32, i32);
type ClosureDbRow = (String, NaiveDate, String);
type SlotDbRow = (
    Uuid,
    String,
    NaiveDate,
    DateTime<Utc>,
    DateTime<Utc>,
    i32,
    i32,
    String,
);
type PolicyDbRow = (i32, i32, i32);

impl BranchView for AdminBookingSlotsView {
    const NAME: &'static str = "admin_booking_slots";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "admin_booking_slots");
    const VERSIONED: bool = true;
    type Data = AdminBookingSlotsData;
    type Filter = AdminBookingSlotsFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();

        async move {
            let rule_row: Option<RuleDbRow> = sqlx::query_as(
                "SELECT branch_id, slot_minutes, capacity, horizon_days \
                 FROM booking_slot_rules WHERE branch_id = $1",
            )
            .bind(branch.as_str())
            .fetch_optional(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let rule = rule_row.map(|(branch_id, slot_minutes, capacity, horizon_days)| {
                AdminSlotRuleDto {
                    branch_id,
                    slot_minutes,
                    capacity,
                    horizon_days,
                }
            });

            let closure_rows: Vec<ClosureDbRow> = sqlx::query_as(
                "SELECT branch_id, closed_on, reason \
                 FROM branch_closures WHERE branch_id = $1 \
                 ORDER BY closed_on ASC",
            )
            .bind(branch.as_str())
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let closures = closure_rows
                .into_iter()
                .map(|(branch_id, closed_on, reason)| AdminClosureDto {
                    branch_id,
                    closed_on: closed_on.to_string(),
                    reason,
                })
                .collect();

            let slot_rows: Vec<SlotDbRow> = sqlx::query_as(
                "SELECT id, branch_id, service_date, start_at, end_at, capacity, reserved, status \
                 FROM booking_slots WHERE branch_id = $1 \
                 ORDER BY start_at ASC",
            )
            .bind(branch.as_str())
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let slots = slot_rows
                .into_iter()
                .map(
                    |(
                        id,
                        branch_id,
                        service_date,
                        start_at,
                        end_at,
                        capacity,
                        reserved,
                        status,
                    )| {
                        AdminSlotDto {
                            id: id.to_string(),
                            branch_id,
                            service_date: service_date.to_string(),
                            start_at: start_at.to_rfc3339(),
                            end_at: end_at.to_rfc3339(),
                            capacity,
                            reserved,
                            status,
                        }
                    },
                )
                .collect();

            let policy_row: Option<PolicyDbRow> = sqlx::query_as(
                "SELECT p.cancel_cutoff_minutes, p.no_show_grace_minutes, p.confirm_expiry_minutes \
                 FROM booking_policies p \
                 JOIN branches b ON b.organization_id = p.organization_id \
                 WHERE b.id = $1",
            )
            .bind(branch.as_str())
            .fetch_optional(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let policy = policy_row.map(
                |(cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes)| {
                    AdminBookingPolicyDto {
                        cancel_cutoff_minutes,
                        no_show_grace_minutes,
                        confirm_expiry_minutes,
                    }
                },
            );

            Ok(AdminBookingSlotsData {
                rule,
                closures,
                slots,
                policy,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(AdminBookingSlotsView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminBookingSlotsData::export_all(&cfg)?;
    AdminBookingSlotsFilter::export_all(&cfg)?;
    Ok(())
}
