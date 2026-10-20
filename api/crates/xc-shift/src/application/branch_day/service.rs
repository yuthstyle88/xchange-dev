//! BranchDayService (CA-7b #1273, design `cash-shift-transfer.md` §4.8, §4.9):
//! the kept branch-day features on `access::Actor`.
//!
//! Every method works on the caller's transaction (§4.9): the CM-1 seam has already locked the
//! branch (`branch_versions … FOR UPDATE`), and its idempotency row commits on the same
//! transaction. A method opens no transaction and takes no lock of its own, and never commits or
//! rolls back: its `Err` rolls the caller's transaction back.
//!
use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::AppResult;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::{Actor, Authorizer, Permission, Refusal, Target};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Side};

use crate::application::shift::service::ShiftService;
use crate::domain::custody::branch_day::BranchDay;
use crate::domain::custody::entities::attendance::Attendance;
use crate::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use crate::domain::custody::entities::stored_event::StoredEvent;
use crate::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use crate::domain::custody::repositories::custody_ports::CustodyPorts;
use crate::domain::custody::repositories::custody_repository::CustodyRepository;
use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::custody_error::CustodyError;
use crate::domain::custody::value_objects::custody_event::EventKind;
use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::cutoff::Cutoff;
use crate::domain::custody::value_objects::phase::Phase;
use crate::domain::shift::repositories::shift_repository::ShiftRepository;
use crate::domain::shift::value_objects::balances::Balances;
use crate::infrastructure::custody::custody_repository::PgCustodyRepository;
use crate::infrastructure::shift::shift_repository::PgShiftRepository;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockDirection {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartService {
    pub equipment_ready: bool,
    pub rates_verified: bool,
    pub rate_updated: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginDraft {
    pub side: Side,
    pub currency: CurrencyCode,
    pub amount: Money,
}

fn map_custody_error(err: CustodyError) -> pos_error::AppError {
    match err {
        CustodyError::RoleNotAllowed => {
            pos_error::AppError::Domain(Refusal::RoleNotAllowed.code().to_string())
        }
        other => pos_error::AppError::Domain(other.to_string()),
    }
}

fn iso_time(ct: CustodyTime) -> String {
    let dt = DateTime::from_timestamp_millis(ct.epoch_millis()).unwrap_or_else(Utc::now);
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn cut_to(s: &str, max_utf16: usize) -> &str {
    let mut utf16 = 0;
    let mut bytes = 0;
    for c in s.chars() {
        let c_u16 = c.len_utf16();
        if utf16 + c_u16 > max_utf16 {
            break;
        }
        utf16 += c_u16;
        bytes += c.len_utf8();
    }
    &s[..bytes]
}

async fn resolve_role(
    ports: &Arc<dyn CustodyPorts>,
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    who: &Actor,
) -> AppResult<CustodyRole> {
    if let Some(role) = ports.employee_role(tx, branch, &who.user_id).await? {
        return Ok(role);
    }
    for sr in &who.roles {
        match sr.role {
            xc_auth::domain::access::Role::BranchManager => return Ok(CustodyRole::Manager),
            xc_auth::domain::access::Role::Teller => return Ok(CustodyRole::Front),
            xc_auth::domain::access::Role::Finance => return Ok(CustodyRole::Finance),
            xc_auth::domain::access::Role::VerificationReviewer => return Ok(CustodyRole::Monitor),
            xc_auth::domain::access::Role::Admin => return Ok(CustodyRole::Owner),
            _ => {}
        }
    }
    Ok(CustodyRole::Other)
}

fn check_permission(
    who: &Actor,
    branch: &BranchId,
    permission: Permission,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let target = Target::new(&who.organization_id, Some(branch.clone()), None);
    Authorizer::check(Some(who), permission, &target, now)
        .map(|_| ())
        .map_err(|r| pos_error::AppError::Domain(r.code().to_string()))
}

pub struct BranchDayService<
    R: CustodyRepository = PgCustodyRepository,
    S: ShiftRepository = PgShiftRepository,
> {
    repo: Arc<R>,
    shifts: ShiftService<S>,
    ports: Arc<dyn CustodyPorts>,
}

impl<R: CustodyRepository, S: ShiftRepository> Clone for BranchDayService<R, S> {
    fn clone(&self) -> Self {
        Self {
            repo: Arc::clone(&self.repo),
            shifts: self.shifts.clone(),
            ports: Arc::clone(&self.ports),
        }
    }
}

impl<R: CustodyRepository, S: ShiftRepository> BranchDayService<R, S> {
    /// No `BranchTx`: the caller's transaction is every method's first argument (§4.9).
    pub fn new(repo: Arc<R>, shifts: ShiftService<S>, ports: Arc<dyn CustodyPorts>) -> Self {
        Self {
            repo,
            shifts,
            ports,
        }
    }

    pub fn shifts(&self) -> &ShiftService<S> {
        &self.shifts
    }

    async fn append_event(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        who: &Actor,
        kind: EventKind,
        at: DateTime<Utc>,
        details: serde_json::Value,
    ) -> AppResult<()> {
        let role = resolve_role(&self.ports, tx, branch, who).await?;
        let event = StoredEvent {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            at,
            actor: who.user_id.clone(),
            role,
            details,
        };
        self.repo.append_events_in(tx, branch, &[event]).await
    }

    pub async fn start_service(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        branch: &BranchId,
        input: StartService,
        now: DateTime<Utc>,
    ) -> AppResult<Phase> {
        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());
        let window = self.ports.service_window(tx, branch, custody_now).await?;
        let utc_offset_minutes = self
            .ports
            .utc_offset_minutes(tx, branch, custody_now)
            .await?;
        let target = Target::new(&who.organization_id, Some(branch.clone()), None);

        let before = self.repo.load_state_in(tx, branch).await?;

        let new_phase =
            BranchDay::start_service(before.phase, who, &target, now, window, utc_offset_minutes)
                .map_err(map_custody_error)?;

        if !input.equipment_ready || !input.rates_verified {
            return Err(map_custody_error(CustodyError::ServiceChecksIncomplete));
        }

        let mut after = before.clone();
        after.phase = new_phase;
        after.service_opened_at = Some(custody_now);

        self.repo
            .store_state_in(tx, branch, &before, &after)
            .await?;

        self.append_event(
            tx,
            branch,
            who,
            EventKind::ServiceOpen,
            now,
            json!({ "rateUpdated": input.rate_updated }),
        )
        .await?;

        Ok(new_phase)
    }

    pub async fn cut_off(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<Phase> {
        let before = self.repo.load_state_in(tx, branch).await?;
        let target = Target::new(&who.organization_id, Some(branch.clone()), None);

        let new_phase =
            BranchDay::cut_off(before.phase, who, &target, now).map_err(map_custody_error)?;

        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());
        let active_shift = match uuid::Uuid::parse_str(&who.user_id) {
            Ok(_) => self.shifts.open_of_teller_in(tx, &who.user_id).await?,
            Err(_) => None,
        };
        let open_shift_id = active_shift.as_ref().map(|s| s.id.as_str());
        let outside = self.ports.outside_facts(tx, branch, open_shift_id).await?;
        let cutoff = Cutoff {
            at: custody_now,
            by: who.user_id.clone(),
            shift_id: open_shift_id.unwrap_or("").to_string(),
            last_ref: outside.last_transaction.clone(),
            pending: outside.pending_transactions.clone(),
        };

        let mut after = before.clone();
        after.phase = new_phase;
        after.cutoff = Some(cutoff.clone());

        self.repo
            .store_state_in(tx, branch, &before, &after)
            .await?;

        self.append_event(
            tx,
            branch,
            who,
            EventKind::CutOff,
            now,
            json!({
                "at": iso_time(custody_now),
                "by": cutoff.by,
                "shiftId": cutoff.shift_id,
                "lastRef": cutoff.last_ref,
                "pending": cutoff.pending,
            }),
        )
        .await?;

        Ok(new_phase)
    }

    pub async fn set_target(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        branch: &BranchId,
        amounts: Balances,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        check_permission(who, branch, Permission::CashTargetSet, now)?;

        let before = self.repo.load_state_in(tx, branch).await?;
        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());

        let target = CashTarget {
            amounts: amounts.clone(),
            by: who.user_id.clone(),
            at: custody_now,
        };

        let mut after = before.clone();
        after.target = Some(target.clone());

        self.repo
            .store_state_in(tx, branch, &before, &after)
            .await?;

        let mut amounts_map = serde_json::Map::new();
        for (code, money) in target.amounts.non_zero() {
            amounts_map.insert(code.as_str().to_string(), json!(money.to_string()));
        }
        self.append_event(
            tx,
            branch,
            who,
            EventKind::CashTarget,
            now,
            json!({
                "amounts": serde_json::Value::Object(amounts_map),
                "by": target.by,
                "at": iso_time(target.at),
            }),
        )
        .await?;

        Ok(())
    }

    pub async fn clock(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        branch: &BranchId,
        direction: ClockDirection,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        check_permission(who, branch, Permission::AttendanceClock, now)?;

        let before = self.repo.load_state_in(tx, branch).await?;
        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());

        let mut after = before.clone();
        match direction {
            ClockDirection::In => {
                let already_clocked_in = after
                    .attendance
                    .iter()
                    .any(|a| a.employee_id == who.user_id && a.clock_out.is_none());
                if !already_clocked_in {
                    let role = self
                        .ports
                        .employee_role(tx, branch, &who.user_id)
                        .await?
                        .unwrap_or(CustodyRole::Front);
                    after.attendance.push(Attendance {
                        id: uuid::Uuid::new_v4().to_string(),
                        employee_id: who.user_id.clone(),
                        role,
                        clock_in: custody_now,
                        clock_out: None,
                    });
                    self.append_event(tx, branch, who, EventKind::ClockIn, now, json!({}))
                        .await?;
                }
            }
            ClockDirection::Out => {
                let open_att = after
                    .attendance
                    .iter_mut()
                    .find(|a| a.employee_id == who.user_id && a.clock_out.is_none());
                let Some(att) = open_att else {
                    return Err(map_custody_error(CustodyError::NoClockInToClose));
                };
                att.clock_out = Some(custody_now);
                self.append_event(tx, branch, who, EventKind::ClockOut, now, json!({}))
                    .await?;
            }
        }

        self.repo
            .store_state_in(tx, branch, &before, &after)
            .await?;
        Ok(())
    }

    pub async fn hold(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        branch: &BranchId,
        customer_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        check_permission(who, branch, Permission::CustomerHoldPlace, now)?;

        if reason.trim().is_empty() {
            return Err(map_custody_error(CustodyError::HoldInvalid));
        }

        let known = self.ports.customer_known(tx, branch, customer_id).await?;
        if !known {
            return Err(map_custody_error(CustodyError::HoldInvalid));
        }

        let before = self.repo.load_state_in(tx, branch).await?;
        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());
        let hold_id = uuid::Uuid::new_v4().to_string();

        let mut after = before.clone();
        after.holds.push(CustomerHold {
            id: hold_id.clone(),
            customer_id: customer_id.to_string(),
            reason: cut_to(reason, 500).to_string(),
            by: who.user_id.clone(),
            at: custody_now,
            status: HoldStatus::Open,
            reviewed_by: None,
            reviewed_at: None,
            note: None,
        });

        self.repo
            .store_state_in(tx, branch, &before, &after)
            .await?;

        self.append_event(
            tx,
            branch,
            who,
            EventKind::HoldCustomer,
            now,
            json!({ "customerId": customer_id }),
        )
        .await?;

        Ok(hold_id)
    }

    pub async fn release(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        branch: &BranchId,
        hold_id: &str,
        note: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        check_permission(who, branch, Permission::VerificationReview, now)?;

        if note.trim().is_empty() {
            return Err(map_custody_error(CustodyError::ReleaseInvalid));
        }

        let before = self.repo.load_state_in(tx, branch).await?;
        let hold = before.holds.iter().find(|h| h.id == hold_id);
        match hold {
            Some(h) if h.status == HoldStatus::Open => {}
            _ => return Err(map_custody_error(CustodyError::ReleaseInvalid)),
        }

        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());
        let mut after = before.clone();
        let h = after.holds.iter_mut().find(|h| h.id == hold_id).unwrap();
        h.status = HoldStatus::Released;
        h.reviewed_by = Some(who.user_id.clone());
        h.reviewed_at = Some(custody_now);
        h.note = Some(cut_to(note, 500).to_string());

        self.repo
            .store_state_in(tx, branch, &before, &after)
            .await?;

        self.append_event(
            tx,
            branch,
            who,
            EventKind::ReleaseCustomer,
            now,
            json!({ "id": hold_id }),
        )
        .await?;

        Ok(())
    }

    pub async fn begin_draft(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        branch: &BranchId,
        input: BeginDraft,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        check_permission(who, branch, Permission::ShiftOperate, now)?;

        if input.amount.is_zero() {
            return Err(map_custody_error(CustodyError::AmountInvalid));
        }
        if input.currency.is_thb() {
            return Err(map_custody_error(CustodyError::DraftInvalid));
        }

        let before = self.repo.load_state_in(tx, branch).await?;

        if before.phase != Phase::Open {
            return Err(map_custody_error(CustodyError::NotCustodianInOpenService));
        }

        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());
        let window = self.ports.service_window(tx, branch, custody_now).await?;
        if !window.allows_begin(custody_now) {
            return Err(map_custody_error(CustodyError::OutsideServiceHours));
        }

        let teller_uuid = match uuid::Uuid::parse_str(&who.user_id) {
            Ok(u) => u,
            Err(_) => return Err(map_custody_error(CustodyError::NoOpenShift)),
        };

        let own_shift: Option<String> = sqlx::query_scalar(
            "SELECT id FROM shifts WHERE branch_id = $1 AND teller_id = $2 AND status = 'OPEN'",
        )
        .bind(branch.as_str())
        .bind(teller_uuid)
        .fetch_optional(&mut **tx)
        .await?;

        let Some(shift_id) = own_shift else {
            return Err(map_custody_error(CustodyError::NoOpenShift));
        };

        let draft_id = uuid::Uuid::new_v4().to_string();

        let mut after = before.clone();
        after.drafts.push(TxDraft {
            id: draft_id.clone(),
            shift_id,
            employee_id: who.user_id.clone(),
            side: input.side,
            currency: input.currency,
            amount: input.amount,
            at: custody_now,
            status: DraftStatus::Open,
            document_ref: None,
        });

        self.repo
            .store_state_in(tx, branch, &before, &after)
            .await?;
        Ok(draft_id)
    }
}
