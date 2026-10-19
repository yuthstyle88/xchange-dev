//! Staff floor reader service (ticket #573, design §3.2).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_kernel::ids::BranchId;
use xc_rate::domain::branch::branch_hours::{BranchHours, HoursWindow};
use xc_shift::domain::custody::entities::customer_hold::HoldStatus;
use xc_shift::domain::custody::repositories::custody_pages::CustodyPages;
use xc_shift::infrastructure::custody::custody_pages::PgCustodyPages;

use super::directory::StaffDirectory;
use crate::presentation::dto::v3::staff_attendance::StaffAttendanceRow;
use crate::presentation::dto::v3::staff_floor::{
    StaffFloor, StaffFloorCutoff, StaffFloorHours, StaffFloorMyShift, StaffFloorTarget,
};
use crate::presentation::dto::v3::staff_holds::StaffHoldRow;

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
        let facts = self.pages.floor(branch, &actor.user_id).await?;
        let now = Utc::now();
        let target = Target::new(&actor.organization_id, Some(branch.clone()), None);

        let phase = facts
            .ops
            .as_ref()
            .map(|o| o.phase.as_str().to_string())
            .unwrap_or_else(|| "CLOSED".to_string());

        let hours = self.read_hours(branch, now).await?;

        let my_clock_in = facts.my_open_attendance.as_ref().map(|a| {
            Utc.timestamp_millis_opt(a.clock_in.epoch_millis())
                .single()
                .unwrap()
                .to_rfc3339()
        });

        let my_shift = self.read_my_shift(branch, &actor.user_id).await?;

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

        let has_cash_permission =
            Authorizer::check(Some(actor), Permission::BranchDayOperate, &target, now).is_ok()
                || Authorizer::check(Some(actor), Permission::CashTargetSet, &target, now).is_ok()
                || Authorizer::check(Some(actor), Permission::CashManage, &target, now).is_ok();

        let (target_dto, safe_dto) = if has_cash_permission {
            let t = facts
                .ops
                .as_ref()
                .and_then(|o| o.target.as_ref())
                .map(|target| {
                    let mut amounts = HashMap::new();
                    for (code, money) in target.amounts.non_zero() {
                        amounts.insert(
                            code.as_str().to_string(),
                            format!(
                                "{:.2}",
                                Decimal::from_i128_with_scale(money.minor_i128(), 2)
                            ),
                        );
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
                        map.insert(
                            code.as_str().to_string(),
                            format!(
                                "{:.2}",
                                Decimal::from_i128_with_scale(money.minor_i128(), 2)
                            ),
                        );
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
        let target = Target::new(&actor.organization_id, Some(branch.clone()), None);
        let can_see_all = Authorizer::check(
            Some(actor),
            Permission::BranchDayOperate,
            &target,
            Utc::now(),
        )
        .is_ok();
        let only = if can_see_all {
            None
        } else {
            Some(actor.user_id.as_str())
        };

        let records = self.pages.attendance(branch, only, before, limit).await?;
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

    async fn read_hours(
        &self,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<Option<StaffFloorHours>> {
        let row: Option<(String, Option<serde_json::Value>)> =
            sqlx::query_as("SELECT timezone, opening_hours FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;

        let (timezone, opening_hours) = match row {
            Some((tz, Some(h))) if !h.is_null() => (tz, h),
            _ => return Ok(None),
        };

        let hours = match BranchHours::new(&timezone, Some(&opening_hours)) {
            Ok(h) => h,
            Err(_) => return Ok(None),
        };

        let tz: Tz = match timezone.parse() {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };

        let today = now.with_timezone(&tz).date_naive();
        match hours.window(today) {
            HoursWindow::Open { open, close } => Ok(Some(StaffFloorHours {
                open: open.to_rfc3339(),
                close: close.to_rfc3339(),
            })),
            _ => Ok(None),
        }
    }

    async fn read_my_shift(
        &self,
        branch: &BranchId,
        user_id: &str,
    ) -> AppResult<Option<StaffFloorMyShift>> {
        let Ok(uuid) = sqlx::types::Uuid::parse_str(user_id) else {
            return Ok(None);
        };
        let row: Option<(String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, opened_at FROM shifts WHERE branch_id = $1 AND teller_id = $2 AND status = 'OPEN'",
        )
        .bind(branch.as_str())
        .bind(uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row.map(|(id, opened_at)| StaffFloorMyShift {
            id,
            opened_at: opened_at.to_rfc3339(),
        }))
    }
}
