//! Staff floor reader service (ticket #573, design §3.2).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::shift_plan::service::ShiftPlanService;
use xc_shift::domain::custody::branch_day::BranchDay;
use xc_shift::domain::custody::entities::customer_hold::HoldStatus;
use xc_shift::domain::custody::repositories::custody_pages::CustodyPages;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::shift::repositories::shift_repository::ShiftRepository;
use xc_shift::domain::shift_plan::shift_plan::ShiftPlan;
use xc_shift::infrastructure::custody::custody_pages::PgCustodyPages;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;

use super::day::local_day_start;
use super::directory::StaffDirectory;
use crate::presentation::dto::v3::common::OpenShiftDto;
use crate::presentation::dto::v3::staff_attendance::StaffAttendanceRow;
use crate::presentation::dto::v3::staff_floor::{
    StaffFloor, StaffFloorCutoff, StaffFloorHours, StaffFloorMyShift, StaffFloorTarget,
};
use crate::presentation::dto::v3::staff_holds::StaffHoldRow;

type OpenShiftRow = (
    String,
    DateTime<Utc>,
    Option<String>,
    Option<String>,
    NaiveDate,
);

pub struct StaffFloorReader {
    pool: PgPool,
    pages: Arc<dyn CustodyPages>,
    directory: StaffDirectory,
}

impl StaffFloorReader {
    pub fn new(pool: PgPool) -> Self {
        let pages = Arc::new(PgCustodyPages::new(pool.clone()));
        let directory = StaffDirectory::new(pool.clone());
        Self {
            pool,
            pages,
            directory,
        }
    }

    pub fn with_pages(
        pool: PgPool,
        pages: Arc<dyn CustodyPages>,
        directory: StaffDirectory,
    ) -> Self {
        Self {
            pool,
            pages,
            directory,
        }
    }

    pub async fn floor(&self, actor: &Actor, branch: &BranchId) -> AppResult<StaffFloor> {
        self.floor_at(actor, branch, Utc::now()).await
    }

    /// The floor as of `now` (CS-5, docs/design/configurable-shifts.md §2.5, §2.7, rows F1–F2): `hours` is the
    /// shift plan's business day holding `now` (`opening_hours` no longer read), and `myShift` also carries
    /// `shiftName`, `businessDate` and `onBreak`. `floor` becomes this at `Utc::now()`.
    pub async fn floor_at(
        &self,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<StaffFloor> {
        let facts = self.pages.floor(branch, &actor.user_id).await?;
        let target = Target::new(&actor.organization_id, Some(branch.clone()), None);

        let plan = self.load_plan(branch).await?;
        let hours = Self::read_hours(&plan, now);

        let phase = match facts.ops.as_ref() {
            None => "NOT_OPENED".to_string(),
            Some(o) if o.phase == Phase::Closed => "NOT_OPENED".to_string(),
            Some(o) if o.phase == Phase::CutOff && facts.open_shifts == 0 => {
                let closed_before_today = o
                    .cutoff
                    .as_ref()
                    .and_then(|c| Utc.timestamp_millis_opt(c.at.epoch_millis()).single())
                    .and_then(|at| plan.business_date_of(at).zip(plan.business_date_of(now)))
                    .is_some_and(|(cutoff_date, now_date)| cutoff_date < now_date);
                if closed_before_today {
                    "NOT_OPENED".to_string()
                } else {
                    "CLOSED".to_string()
                }
            }
            Some(o) => BranchDay::read_phase(o.phase, facts.open_shifts)
                .as_str()
                .to_string(),
        };

        let my_clock_in = facts.my_open_attendance.as_ref().map(|a| {
            Utc.timestamp_millis_opt(a.clock_in.epoch_millis())
                .single()
                .unwrap()
                .to_rfc3339()
        });

        let my_shift = self
            .read_my_shift(branch, &actor.user_id, &plan, now)
            .await?;

        let cutoff = facts
            .ops
            .as_ref()
            .and_then(|o| o.cutoff.as_ref())
            .map(|c| StaffFloorCutoff {
                at: Utc
                    .timestamp_millis_opt(c.at.epoch_millis())
                    .single()
                    .unwrap()
                    .to_rfc3339(),
                pending_count: c.pending.len(),
            });

        let has_cash_manage =
            Authorizer::check(Some(actor), Permission::CashManage, &target, now).is_ok();
        let has_cash_permission =
            Authorizer::check(Some(actor), Permission::BranchDayOperate, &target, now).is_ok()
                || Authorizer::check(Some(actor), Permission::CashTargetSet, &target, now).is_ok()
                || has_cash_manage;

        let open_shifts = if has_cash_manage {
            PgShiftRepository::new(self.pool.clone())
                .open_of_branch(branch)
                .await?
                .into_iter()
                .map(OpenShiftDto::from)
                .collect()
        } else {
            Vec::new()
        };

        let (target_dto, safe_dto) = if has_cash_permission {
            let minor_units: HashMap<String, i16> =
                sqlx::query_as("SELECT trim(code), minor_unit FROM currencies")
                    .fetch_all(&self.pool)
                    .await?
                    .into_iter()
                    .collect();
            let fmt = |code: &str, mut d: rust_decimal::Decimal| {
                d = d.normalize();
                let scale = minor_units.get(code).copied().unwrap_or(2).max(0) as u32;
                if d.scale() < scale {
                    d.rescale(scale);
                }
                d.to_string()
            };
            let t = facts
                .ops
                .as_ref()
                .and_then(|o| o.target.as_ref())
                .map(|target| {
                    let mut amounts = HashMap::new();
                    for (code, money) in target.amounts.non_zero() {
                        amounts
                            .insert(code.as_str().to_string(), fmt(code.as_str(), money.value()));
                    }
                    StaffFloorTarget {
                        amounts,
                        at: Utc
                            .timestamp_millis_opt(target.at.epoch_millis())
                            .single()
                            .unwrap()
                            .to_rfc3339(),
                    }
                });
            let s = crate::application::compat::snapshot::ledger_safe(&self.pool, branch)
                .await?
                .map(|safe| {
                    let mut map = HashMap::new();
                    for (code, money) in safe.non_zero() {
                        map.insert(code.as_str().to_string(), fmt(code.as_str(), money.value()));
                    }
                    map
                });
            (t, s)
        } else {
            (None, None)
        };

        Ok(StaffFloor {
            phase,
            hours,
            my_clock_in,
            my_shift,
            cutoff,
            target: target_dto,
            safe: safe_dto,
            open_shifts,
        })
    }

    pub async fn holds(
        &self,
        _actor: &Actor,
        branch: &BranchId,
        before: Option<&str>,
        limit: u16,
    ) -> AppResult<Vec<StaffHoldRow>> {
        let records = self.pages.holds(branch, before, limit).await?;
        let mut rows = Vec::with_capacity(records.len());
        for r in records {
            let status = match r.status {
                HoldStatus::Open => "OPEN".to_string(),
                HoldStatus::Released => "RELEASED".to_string(),
            };
            let at = Utc
                .timestamp_millis_opt(r.at.epoch_millis())
                .single()
                .unwrap()
                .to_rfc3339();
            rows.push(StaffHoldRow {
                id: r.id,
                customer_id: r.customer_id,
                customer_name: None,
                reason: r.reason,
                status,
                at,
            });
        }
        Ok(rows)
    }

    pub async fn attendance(
        &self,
        actor: &Actor,
        branch: &BranchId,
        before: Option<&str>,
        limit: u16,
    ) -> AppResult<Vec<StaffAttendanceRow>> {
        let now = Utc::now();
        let target = Target::new(&actor.organization_id, Some(branch.clone()), None);
        let can_see_all =
            Authorizer::check(Some(actor), Permission::BranchDayOperate, &target, now).is_ok();
        let only = if can_see_all {
            None
        } else {
            Some(actor.user_id.as_str())
        };

        let tz: Tz = sqlx::query_scalar::<_, String>("SELECT timezone FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?
            .and_then(|s| s.parse().ok())
            .unwrap_or(Tz::Asia__Bangkok);
        let since = CustodyTime::from_epoch_millis(local_day_start(tz, now).timestamp_millis());

        let records = self
            .pages
            .attendance_since(branch, only, since, before, limit)
            .await?;
        let user_ids: Vec<&str> = records.iter().map(|r| r.employee_id.as_str()).collect();
        let names = self.directory.names(&user_ids).await?;

        let mut rows = Vec::with_capacity(records.len());
        for r in records {
            let name = names.get(&r.employee_id).cloned().unwrap_or_default();
            let in_time = Utc
                .timestamp_millis_opt(r.clock_in.epoch_millis())
                .single()
                .unwrap()
                .to_rfc3339();
            let out_time = r.clock_out.map(|t| {
                Utc.timestamp_millis_opt(t.epoch_millis())
                    .single()
                    .unwrap()
                    .to_rfc3339()
            });
            rows.push(StaffAttendanceRow {
                id: r.id,
                name,
                in_time,
                out_time,
            });
        }
        Ok(rows)
    }

    async fn load_plan(&self, branch: &BranchId) -> AppResult<ShiftPlan> {
        let branch_tx = BranchTx::new(Transactor::new(self.pool.clone()));
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
        let plan = ShiftPlanService::new(Arc::new(PgShiftTemplateStore), branch_tx)
            .plan_in(&mut tx, branch)
            .await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(plan)
    }

    fn read_hours(plan: &ShiftPlan, now: DateTime<Utc>) -> Option<StaffFloorHours> {
        plan.span_for(now).map(|(open, close)| StaffFloorHours {
            open: open.to_rfc3339(),
            close: close.to_rfc3339(),
        })
    }

    async fn read_my_shift(
        &self,
        branch: &BranchId,
        user_id: &str,
        plan: &ShiftPlan,
        now: DateTime<Utc>,
    ) -> AppResult<Option<StaffFloorMyShift>> {
        let Ok(uuid) = sqlx::types::Uuid::parse_str(user_id) else {
            return Ok(None);
        };
        let row: Option<OpenShiftRow> = sqlx::query_as(
            "SELECT id, opened_at, template_id::text, template_name, business_date \
               FROM shifts WHERE branch_id = $1 AND teller_id = $2 AND status = 'OPEN'",
        )
        .bind(branch.as_str())
        .bind(uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row.map(
            |(id, opened_at, template_id, template_name, business_date)| StaffFloorMyShift {
                id,
                opened_at: opened_at.to_rfc3339(),
                shift_name: template_name,
                business_date: Some(business_date.to_string()),
                on_break: Some(
                    template_id
                        .as_deref()
                        .is_some_and(|tid| plan.on_break(tid, now)),
                ),
            },
        ))
    }
}
