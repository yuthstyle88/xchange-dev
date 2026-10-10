//! Custody service application service.

use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::application::custody::request::parse_command;
use crate::application::shift::service::ShiftService;
use crate::domain::custody::commands::custody_command::CustodyCommand;
use crate::domain::custody::entities::custody_machine::CustodyMachine;
use crate::domain::custody::entities::stored_event::StoredEvent;
use crate::domain::custody::repositories::custody_ports::CustodyPorts;
use crate::domain::custody::repositories::custody_repository::CustodyRepository;
use crate::domain::custody::state::CustodyState;
use crate::domain::custody::value_objects::custody_actor::CustodyActor;
use crate::domain::custody::value_objects::custody_event::{CustodyEvent, EventKind};
use crate::domain::custody::value_objects::custody_facts::{
    CustodyFacts, OpenShift as FactOpenShift,
};
use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::request_kind::RequestKind;
use crate::domain::custody::value_objects::save_attempt::SaveAttempt;
use crate::domain::custody::value_objects::shift_effect::ShiftEffect;
use crate::domain::shift::commands::close_shift::CloseShift;
use crate::domain::shift::commands::move_stock::MoveStock;
use crate::domain::shift::commands::open_shift::OpenShift;
use crate::domain::shift::repositories::shift_repository::ShiftRepository;
use crate::domain::shift::value_objects::stock_delta::Direction;
use crate::infrastructure::custody::custody_repository::PgCustodyRepository;
use crate::infrastructure::shift::shift_repository::PgShiftRepository;

fn truncate_millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .unwrap_or(t)
}

fn role_to_str(role: CustodyRole) -> &'static str {
    match role {
        CustodyRole::Owner => "OWNER",
        CustodyRole::Front => "FRONT",
        CustodyRole::Finance => "FINANCE",
        CustodyRole::Monitor => "MONITOR",
        CustodyRole::Manager => "MANAGER",
        CustodyRole::Backend => "BACKEND",
        CustodyRole::Other => "OTHER",
    }
}

fn iso_time(ct: CustodyTime) -> String {
    let dt = DateTime::from_timestamp_millis(ct.epoch_millis()).unwrap_or_else(Utc::now);
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn event_details(
    ev: &CustodyEvent,
    actor: &CustodyActor,
    facts: &CustodyFacts,
    new_state: &CustodyState,
) -> Value {
    match ev.kind {
        EventKind::SetEntity => {
            if let Some(ref entity) = new_state.entity {
                json!({
                    "company": entity.company,
                    "site": entity.site,
                    "confirmedBy": entity.confirmed_by,
                    "at": iso_time(entity.at),
                })
            } else {
                json!({})
            }
        }
        EventKind::Enable | EventKind::ClockIn | EventKind::ClockOut => {
            json!({})
        }
        EventKind::CashTarget => {
            if let Some(ref target) = new_state.target {
                let mut amounts = serde_json::Map::new();
                for (code, money) in target.amounts.non_zero() {
                    amounts.insert(code.as_str().to_string(), json!(money.to_string()));
                }
                json!({
                    "amounts": Value::Object(amounts),
                    "by": target.by,
                    "at": iso_time(target.at),
                })
            } else {
                json!({})
            }
        }
        EventKind::ServiceOpen => {
            json!({
                "rateUpdated": facts.rates_updated,
            })
        }
        EventKind::CutOff => {
            if let Some(ref cutoff) = new_state.cutoff {
                json!({
                    "at": iso_time(cutoff.at),
                    "by": cutoff.by,
                    "shiftId": cutoff.shift_id,
                    "lastRef": cutoff.last_ref,
                    "pending": cutoff.pending,
                })
            } else {
                json!({})
            }
        }
        EventKind::Propose | EventKind::Commit => {
            let req = new_state
                .requests
                .iter()
                .find(|r| Some(&r.id) == ev.subject.as_ref());
            let kind_str = req.map(|r| r.kind.as_str()).unwrap_or("");
            json!({
                "id": ev.subject,
                "kind": kind_str,
            })
        }
        EventKind::HoldCustomer => {
            json!({
                "customerId": ev.subject,
            })
        }
        EventKind::ReleaseCustomer | EventKind::Reject | EventKind::Reconcile => {
            json!({
                "id": ev.subject,
            })
        }
        EventKind::Sign => {
            json!({
                "id": ev.subject,
                "role": role_to_str(actor.role),
            })
        }
        EventKind::Dispatch => {
            let req = new_state
                .requests
                .iter()
                .find(|r| Some(&r.id) == ev.subject.as_ref());
            let kind_str = req.map(|r| r.kind.as_str()).unwrap_or("");
            let curr_str = req
                .and_then(|r| r.currency)
                .map(|c| c.as_str())
                .unwrap_or("");
            let amt_str = req
                .and_then(|r| r.amount)
                .map(|m| m.to_string())
                .unwrap_or_default();
            json!({
                "id": ev.subject,
                "kind": kind_str,
                "currency": curr_str,
                "amount": amt_str,
            })
        }
    }
}

pub struct CustodyService<
    R: CustodyRepository = PgCustodyRepository,
    S: ShiftRepository = PgShiftRepository,
> {
    repo: Arc<R>,
    shifts: ShiftService<S>,
    ports: Arc<dyn CustodyPorts>,
    branch_tx: BranchTx,
}

impl<R: CustodyRepository, S: ShiftRepository> Clone for CustodyService<R, S> {
    fn clone(&self) -> Self {
        Self {
            repo: Arc::clone(&self.repo),
            shifts: self.shifts.clone(),
            ports: Arc::clone(&self.ports),
            branch_tx: self.branch_tx.clone(),
        }
    }
}

impl<R: CustodyRepository, S: ShiftRepository> CustodyService<R, S> {
    pub fn new(
        repo: Arc<R>,
        shifts: ShiftService<S>,
        ports: Arc<dyn CustodyPorts>,
        branch_tx: BranchTx,
    ) -> Self {
        Self {
            repo,
            shifts,
            ports,
            branch_tx,
        }
    }

    pub async fn state(&self, branch: &BranchId) -> AppResult<CustodyState> {
        self.repo.load_state(branch).await
    }

    pub async fn events(&self, branch: &BranchId) -> AppResult<Vec<StoredEvent>> {
        self.repo.list_events(branch).await
    }

    pub async fn check_direct_shift_action_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
    ) -> AppResult<()> {
        let enabled = self.repo.is_enabled_in(tx, branch).await?;
        if enabled {
            return Err(AppError::Domain(
                "ใช้ขั้นตอนรับ–ส่งเงินและการลงชื่อในเมนูปฏิบัติงาน".into(),
            ));
        }
        Ok(())
    }

    pub async fn check_save_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        save: &SaveAttempt,
        actor: &CustodyActor,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let state = self.repo.load_state_in(tx, branch).await?;
        if !state.enabled {
            return Ok(());
        }
        let active_shift = self.shifts.active_in(tx, branch).await?;
        let open_shift_id = active_shift.as_ref().map(|s| s.id.as_str());
        let outside = self.ports.outside_facts(tx, branch, open_shift_id).await?;
        let open_shift = active_shift.map(|s| FactOpenShift {
            id: s.id,
            expected: s.expected,
        });
        let now_trunc = truncate_millis(now);
        let custody_now = CustodyTime::from_epoch_millis(now_trunc.timestamp_millis());
        let service_window = self.ports.service_window(tx, branch, custody_now).await?;
        let utc_offset_minutes = self
            .ports
            .utc_offset_minutes(tx, branch, custody_now)
            .await?;
        let facts = CustodyFacts {
            service_window,
            utc_offset_minutes,
            open_shift,
            has_live_rate: outside.has_live_rate,
            rates_updated: outside.rates_updated,
            pending_transactions: outside.pending_transactions,
            last_transaction: outside.last_transaction,
            customer_known: false,
            new_id: uuid::Uuid::new_v4().to_string(),
        };
        CustodyMachine::check_save(&state, save, actor, &facts, custody_now)
            .map_err(|e| AppError::Domain(e.to_string()))
    }

    pub async fn complete_draft_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        save: &SaveAttempt,
        document_ref: &str,
    ) -> AppResult<()> {
        let state = self.repo.load_state_in(tx, branch).await?;
        let new_state = CustodyMachine::complete_draft(&state, save, document_ref);
        self.repo
            .store_state_in(tx, branch, &state, &new_state)
            .await?;
        Ok(())
    }

    pub async fn command(
        &self,
        branch: &BranchId,
        action: &str,
        payload: &Value,
        actor: &CustodyActor,
        now: DateTime<Utc>,
    ) -> AppResult<Value> {
        let cmd = parse_command(action, payload)?;

        if !self.repo.branch_exists(branch).await? {
            return Err(AppError::Domain("ไม่พบสาขาที่เลือก กรุณารีเฟรช".into()));
        }

        let this = self.clone();
        let b = branch.clone();
        let actor = actor.clone();
        let action = action.to_owned();
        let now = truncate_millis(now);

        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { this.command_in(tx, &b, &action, cmd, &actor, now).await })
            })
            .await
    }

    async fn command_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        action: &str,
        cmd: CustodyCommand,
        actor: &CustodyActor,
        now: DateTime<Utc>,
    ) -> AppResult<Value> {
        if !matches!(
            cmd,
            CustodyCommand::SetEntity { .. } | CustodyCommand::Enable
        ) {
            if let Some(ref employee_id) = actor.employee_id {
                if self
                    .ports
                    .employee_role(tx, branch, employee_id)
                    .await?
                    .is_none()
                {
                    return Err(AppError::Unauthenticated);
                }
            }
        }

        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());
        if let CustodyCommand::Propose(ref proposal) = cmd {
            if let Some(ref receiver) = proposal.receiver {
                let role_opt = self.ports.employee_role(tx, branch, receiver).await?;
                let allowed = match (proposal.kind, role_opt) {
                    (RequestKind::Handover, Some(CustodyRole::Front)) => true,
                    (k, Some(CustodyRole::Front | CustodyRole::Manager | CustodyRole::Finance))
                        if k != RequestKind::Handover =>
                    {
                        true
                    }
                    _ => false,
                };
                if !allowed {
                    return Err(AppError::Domain("ผู้รับหรือบทบาทไม่ถูกต้องสำหรับสาขา".into()));
                }
            }
            let owned = self
                .ports
                .evidence_owned(tx, branch, &actor.id, &proposal.evidence_keys)
                .await?;
            if !owned {
                return Err(AppError::Domain("หลักฐานไม่ใช่ของผู้ส่งหรือสาขานี้".into()));
            }
        }

        let state = self.repo.load_state_in(tx, branch).await?;

        let active_shift = self.shifts.active_in(tx, branch).await?;
        let open_shift_id = active_shift.as_ref().map(|s| s.id.as_str());
        let outside = self.ports.outside_facts(tx, branch, open_shift_id).await?;
        let open_shift = active_shift.map(|s| FactOpenShift {
            id: s.id,
            expected: s.expected,
        });

        let customer_known = match &cmd {
            CustodyCommand::Hold { customer_id, .. } => {
                self.ports.customer_known(tx, branch, customer_id).await?
            }
            _ => false,
        };

        let new_id = uuid::Uuid::new_v4().to_string();

        let service_window = self.ports.service_window(tx, branch, custody_now).await?;
        let utc_offset_minutes = self
            .ports
            .utc_offset_minutes(tx, branch, custody_now)
            .await?;
        let facts = CustodyFacts {
            service_window,
            utc_offset_minutes,
            open_shift,
            has_live_rate: outside.has_live_rate,
            rates_updated: outside.rates_updated,
            pending_transactions: outside.pending_transactions,
            last_transaction: outside.last_transaction,
            customer_known,
            new_id,
        };

        let transition = CustodyMachine::apply(&state, &cmd, actor, &facts, custody_now)
            .map_err(|e| AppError::Domain(e.to_string()))?;

        let shift_actor = actor.employee_id.as_deref().unwrap_or(&actor.id);

        for effect in &transition.effects {
            match effect {
                ShiftEffect::OpenShift {
                    request_id,
                    opening,
                } => {
                    let req_id = request_id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    let mut opening_map = serde_json::Map::new();
                    for (c, m) in opening.non_zero() {
                        opening_map.insert(c.as_str().to_string(), json!(m.to_string()));
                    }
                    let payload = json!({
                        "requestId": req_id,
                        "opening": Value::Object(opening_map),
                    });
                    let open_cmd = OpenShift::from_payload(&payload).map_err(AppError::from)?;
                    self.shifts
                        .open_in(tx, branch, open_cmd, shift_actor, now)
                        .await?;
                }
                ShiftEffect::CloseShift {
                    request_id,
                    shift_id,
                    expected,
                    counted,
                    note,
                } => {
                    let mut exp_map = serde_json::Map::new();
                    for (c, m) in expected.non_zero() {
                        exp_map.insert(c.as_str().to_string(), json!(m.to_string()));
                    }
                    let mut cnt_map = serde_json::Map::new();
                    for (c, m) in counted.non_zero() {
                        cnt_map.insert(c.as_str().to_string(), json!(m.to_string()));
                    }
                    let payload = json!({
                        "requestId": request_id,
                        "shiftId": shift_id,
                        "expected": Value::Object(exp_map),
                        "counted": Value::Object(cnt_map),
                        "note": note,
                    });
                    let close_cmd = CloseShift::from_payload(&payload).map_err(AppError::from)?;
                    self.shifts
                        .close_in(tx, branch, close_cmd, shift_actor, now)
                        .await?;
                }
                ShiftEffect::MoveStock {
                    request_id,
                    shift_id,
                    currency,
                    direction,
                    amount,
                    reason,
                } => {
                    let req_id = request_id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    let dir_str = match direction {
                        Direction::In => "IN",
                        Direction::Out => "OUT",
                    };
                    let payload = json!({
                        "requestId": req_id,
                        "shiftId": shift_id,
                        "currency": currency.as_str(),
                        "direction": dir_str,
                        "amount": amount.to_string(),
                        "reason": reason,
                    });
                    let move_cmd = MoveStock::from_payload(&payload).map_err(AppError::from)?;
                    self.shifts
                        .move_stock_in(tx, branch, move_cmd, shift_actor, now)
                        .await?;
                }
            }
        }

        self.repo
            .store_state_in(tx, branch, &state, &transition.state)
            .await?;

        let mut stored_events = Vec::with_capacity(transition.events.len());
        for ev in &transition.events {
            let details = event_details(ev, actor, &facts, &transition.state);
            stored_events.push(StoredEvent {
                id: uuid::Uuid::new_v4().to_string(),
                kind: ev.kind,
                at: now,
                actor: shift_actor.to_string(),
                role: actor.role,
                details,
            });
        }
        if !stored_events.is_empty() {
            self.repo
                .append_events_in(tx, branch, &stored_events)
                .await?;
        }

        let res = match action {
            "opsBeginTransaction" => json!({ "id": transition.id }),
            "opsPropose" => json!({ "saved": true, "id": transition.id }),
            _ => json!({ "saved": true }),
        };

        Ok(res)
    }
}
