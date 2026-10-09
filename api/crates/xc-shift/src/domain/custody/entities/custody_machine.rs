//! Custody state machine.

use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};

use crate::domain::custody::commands::custody_command::{ClockDirection, CustodyCommand};
use crate::domain::custody::commands::proposal::Proposal;
use crate::domain::custody::entities::attendance::Attendance;
use crate::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use crate::domain::custody::entities::custody_request::{CustodyRequest, Decision};
use crate::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use crate::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use crate::domain::custody::state::CustodyState;
use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::count_sheet::CountSheet;
use crate::domain::custody::value_objects::custody_actor::CustodyActor;
use crate::domain::custody::value_objects::custody_error::CustodyError;
use crate::domain::custody::value_objects::custody_event::{CustodyEvent, EventKind};
use crate::domain::custody::value_objects::custody_facts::CustodyFacts;
use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::cutoff::Cutoff;
use crate::domain::custody::value_objects::entity::Entity;
use crate::domain::custody::value_objects::phase::Phase;
use crate::domain::custody::value_objects::request_kind::RequestKind;
use crate::domain::custody::value_objects::request_status::{
    Reconciliation, RequestStatus, TransferStage,
};
use crate::domain::custody::value_objects::save_attempt::{PriorSave, SaveAttempt};
use crate::domain::custody::value_objects::shift_effect::ShiftEffect;
use crate::domain::custody::value_objects::signature::Signature;
use crate::domain::custody::value_objects::transition::Transition;
use crate::domain::shift::entities::shift_ledger::ShiftLedger;
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::stock_delta::{Direction, StockDelta};

pub struct CustodyMachine;

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
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

fn looks_like_an_id(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f' | '-'))
}

fn movement_reason(kind: RequestKind, note_or_id: &str) -> String {
    format!("{} · {}", kind.as_str(), note_or_id)
}

fn is_clocked_in(state: &CustodyState, employee_id: &str) -> bool {
    state
        .attendance
        .iter()
        .any(|a| a.employee_id == employee_id && a.clock_out.is_none())
}

fn has_non_zero(opt: &Option<StockDelta>) -> bool {
    opt.as_ref().is_some_and(|d| !d.non_zero().is_empty())
}

/// The draft a save names and can finish: the one it names, still open, for
/// exactly the figures it saved. Who opened the draft and which shift it
/// belongs to are not part of the match (ticket #154, decision B).
fn draft_matches_save(draft: &TxDraft, save: &SaveAttempt) -> bool {
    save.draft_id.as_deref() == Some(draft.id.as_str())
        && draft.status == DraftStatus::Open
        && draft.side == save.side
        && draft.currency == save.currency
        && draft.amount == save.amount
}

fn validate_count_sheet(sheet: &CountSheet) -> Result<(), CustodyError> {
    if sheet.denominations.is_empty() || sheet.denominations.len() > 200 {
        return Err(CustodyError::DenominationsRequired);
    }
    let max_denom = Money::from_minor(10_000_000).map_err(|_| CustodyError::AmountInvalid)?;
    let the_seven = &CurrencyCode::all()[..7];
    for row in &sheet.denominations {
        if !the_seven.contains(&row.currency)
            || row.denomination.is_zero()
            || row.denomination > max_denom
            || row.count > 1_000_000
        {
            return Err(CustodyError::DenominationInvalid);
        }
    }
    for &curr in the_seven {
        let mut sum: i128 = 0;
        for row in &sheet.denominations {
            if row.currency == curr {
                sum += (row.denomination.minor_i128()) * (row.count as i128);
            }
        }
        if sum != sheet.totals.get(curr).minor_i128() {
            return Err(CustodyError::DenominationMismatch(curr));
        }
    }
    Ok(())
}

impl CustodyMachine {
    pub fn apply(
        state: &CustodyState,
        cmd: &CustodyCommand,
        actor: &CustodyActor,
        facts: &CustodyFacts,
        now: CustodyTime,
    ) -> Result<Transition, CustodyError> {
        // 8.1.1: SetEntity and Enable are handled first.
        match cmd {
            CustodyCommand::SetEntity {
                company,
                site,
                confirm_separated,
            } => {
                if actor.role != CustodyRole::Owner {
                    return Err(CustodyError::RoleNotAllowed);
                }
                if state.enabled || facts.open_shift.is_some() {
                    return Err(CustodyError::EntityLocked);
                }
                let comp_trim = company.trim();
                let site_trim = site.trim();
                if comp_trim.is_empty()
                    || site_trim.is_empty()
                    || utf16_len(comp_trim) > 150
                    || utf16_len(site_trim) > 100
                    || !confirm_separated
                {
                    return Err(CustodyError::EntityIncomplete);
                }
                let mut new_state = state.clone();
                new_state.entity = Some(Entity {
                    company: comp_trim.to_string(),
                    site: site_trim.to_string(),
                    confirmed_by: actor.id.clone(),
                    at: now,
                });
                return Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::SetEntity,
                        subject: None,
                    }],
                    effects: Vec::new(),
                    id: None,
                });
            }
            CustodyCommand::Enable => {
                if actor.role != CustodyRole::Owner {
                    return Err(CustodyError::RoleNotAllowed);
                }
                if facts.open_shift.is_some() {
                    return Err(CustodyError::ShiftStillOpen);
                }
                if state.entity.is_none() {
                    return Err(CustodyError::EntityMissing);
                }
                let mut new_state = state.clone();
                new_state.enabled = true;
                return Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::Enable,
                        subject: None,
                    }],
                    effects: Vec::new(),
                    id: None,
                });
            }
            _ => {}
        }

        // 8.1.2: Any other command while state.enabled is false -> NotEnabled.
        if !state.enabled {
            return Err(CustodyError::NotEnabled);
        }

        // 8.1.3: Clock is handled next. It needs an employee, not a clock-in.
        if let CustodyCommand::Clock(dir) = cmd {
            let me = match &actor.employee_id {
                Some(id) => id,
                None => return Err(CustodyError::EmployeeRequired),
            };
            match dir {
                ClockDirection::In => {
                    if is_clocked_in(state, me) {
                        return Ok(Transition {
                            state: state.clone(),
                            events: Vec::new(),
                            effects: Vec::new(),
                            id: None,
                        });
                    }
                    let mut new_state = state.clone();
                    new_state.attendance.push(Attendance {
                        id: facts.new_id.clone(),
                        employee_id: me.clone(),
                        role: actor.role,
                        clock_in: now,
                        clock_out: None,
                    });
                    return Ok(Transition {
                        state: new_state,
                        events: vec![CustodyEvent {
                            kind: EventKind::ClockIn,
                            subject: None,
                        }],
                        effects: Vec::new(),
                        id: None,
                    });
                }
                ClockDirection::Out => {
                    if !is_clocked_in(state, me) {
                        return Err(CustodyError::NoClockInToClose);
                    }
                    if state.custodian.as_deref() == Some(me.as_str())
                        || state.requests.iter().any(|r| {
                            r.status == RequestStatus::Pending
                                && (r.sender == *me || r.receiver.as_deref() == Some(me.as_str()))
                        })
                    {
                        return Err(CustodyError::DutiesOutstanding);
                    }
                    let mut new_state = state.clone();
                    for att in &mut new_state.attendance {
                        if att.employee_id == *me && att.clock_out.is_none() {
                            att.clock_out = Some(now);
                            break;
                        }
                    }
                    return Ok(Transition {
                        state: new_state,
                        events: vec![CustodyEvent {
                            kind: EventKind::ClockOut,
                            subject: None,
                        }],
                        effects: Vec::new(),
                        id: None,
                    });
                }
            }
        }

        // 8.1.4: Any other command: actor.employee_id is None, or I am not clocked in -> NotClockedIn.
        let me = match &actor.employee_id {
            Some(id) if is_clocked_in(state, id) => id.as_str(),
            _ => return Err(CustodyError::NotClockedIn),
        };

        // 8.1.5: Reject, Sign, Dispatch, Commit, Reconcile: find request before any role check.
        match cmd {
            CustodyCommand::Reject { id, reason } => {
                let req = state
                    .requests
                    .iter()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                // (a) request stage is InTransit -> MoneyInTransit (before role check!)
                if req.stage == Some(TransferStage::InTransit) {
                    return Err(CustodyError::MoneyInTransit);
                }
                // (b) role, else RoleNotAllowed (Monitor or Backend)
                if !matches!(actor.role, CustodyRole::Monitor | CustodyRole::Backend) {
                    return Err(CustodyError::RoleNotAllowed);
                }
                // (c) status not Pending -> RequestFinished
                if req.status != RequestStatus::Pending {
                    return Err(CustodyError::RequestFinished);
                }
                // (d) reason blank -> RejectReasonRequired
                if reason.trim().is_empty() {
                    return Err(CustodyError::RejectReasonRequired);
                }
                let mut new_state = state.clone();
                let r = new_state
                    .requests
                    .iter_mut()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                r.status = RequestStatus::Rejected;
                r.rejected = Some(Decision {
                    by: me.to_string(),
                    at: now,
                    text: cut_to(reason, 500).to_string(),
                });
                if new_state.phase == Phase::Counting {
                    new_state.phase = Phase::CutOff;
                }
                return Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::Reject,
                        subject: Some(id.clone()),
                    }],
                    effects: Vec::new(),
                    id: None,
                });
            }
            CustodyCommand::Sign { id } => {
                let req = state
                    .requests
                    .iter()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                // 8.4 Sign rules:
                // 1. Request status is not Pending -> RequestFinished
                if req.status != RequestStatus::Pending {
                    return Err(CustodyError::RequestFinished);
                }
                // 2. The request is a transfer, I am its receiver, and stage is not InTransit -> AwaitingDispatch
                if req.kind.is_transfer()
                    && req.receiver.as_deref() == Some(me)
                    && req.stage != Some(TransferStage::InTransit)
                {
                    return Err(CustodyError::AwaitingDispatch);
                }
                // 3. The request already has a signature with my employee_id -> accepted, nothing changes, no event
                if req.signatures.iter().any(|s| s.employee_id == me) {
                    return Ok(Transition {
                        state: state.clone(),
                        events: Vec::new(),
                        effects: Vec::new(),
                        id: None,
                    });
                }
                // 4. Role Front: must be receiver and kind Handover or transfer
                if actor.role == CustodyRole::Front
                    && !(req.receiver.as_deref() == Some(me)
                        && (req.kind == RequestKind::Handover || req.kind.is_transfer()))
                {
                    return Err(CustodyError::NotTheReceiver);
                }
                // 5. Role Finance: kind must be a transfer
                if actor.role == CustodyRole::Finance && !req.kind.is_transfer() {
                    return Err(CustodyError::NotAFinanceRequest);
                }
                // 6. Role Owner or Other -> SignNotAllowed
                if matches!(actor.role, CustodyRole::Owner | CustodyRole::Other) {
                    return Err(CustodyError::SignNotAllowed);
                }
                // 7. Role Backend -> BackendCommitsInstead
                if actor.role == CustodyRole::Backend {
                    return Err(CustodyError::BackendCommitsInstead);
                }
                // 8. Accepted
                let mut new_state = state.clone();
                let r = new_state
                    .requests
                    .iter_mut()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                r.signatures.push(Signature {
                    employee_id: me.to_string(),
                    role: actor.role,
                    at: now,
                });
                return Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::Sign,
                        subject: Some(id.clone()),
                    }],
                    effects: Vec::new(),
                    id: None,
                });
            }
            CustodyCommand::Dispatch { id } => {
                let req = state
                    .requests
                    .iter()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                if actor.role != CustodyRole::Backend {
                    return Err(CustodyError::RoleNotAllowed);
                }
                // (a) status not Pending, or kind is not a transfer -> NotDispatchable
                if req.status != RequestStatus::Pending || !req.kind.is_transfer() {
                    return Err(CustodyError::NotDispatchable);
                }
                // (b) stage already InTransit -> accepted, nothing changes, no event, no effect
                if req.stage == Some(TransferStage::InTransit) {
                    return Ok(Transition {
                        state: state.clone(),
                        events: Vec::new(),
                        effects: Vec::new(),
                        id: None,
                    });
                }
                // (c) open shift id differs from request's shift_id (or no open shift) -> ShiftChanged
                if facts.open_shift.as_ref().map(|s| &s.id) != req.shift_id.as_ref() {
                    return Err(CustodyError::ShiftChanged);
                }
                // (d) kind SafeOut and safe.get(currency) < amount -> SafeInsufficient
                let currency = req.currency.ok_or(CustodyError::NotDispatchable)?;
                let amount = req.amount.ok_or(CustodyError::NotDispatchable)?;
                let shift_id = req.shift_id.clone().ok_or(CustodyError::NotDispatchable)?;
                if req.kind == RequestKind::SafeOut {
                    let safe = state.safe.as_ref().ok_or(CustodyError::SafeNotOpened)?;
                    if safe.get(currency) < amount {
                        return Err(CustodyError::SafeInsufficient);
                    }
                }
                let mut new_state = state.clone();
                let mut effects = Vec::new();
                if matches!(req.kind, RequestKind::SafeIn | RequestKind::ExternalOut) {
                    effects.push(ShiftEffect::MoveStock {
                        request_id: Some(req.request_id.clone()),
                        shift_id,
                        currency,
                        direction: Direction::Out,
                        amount,
                        reason: format!("IN_TRANSIT · {} · {}", req.kind.as_str(), req.id),
                    });
                } else if req.kind == RequestKind::SafeOut {
                    let safe = new_state.safe.as_mut().ok_or(CustodyError::SafeNotOpened)?;
                    let curr_money = safe.get(currency);
                    let new_money = MoneyDelta::from(curr_money)
                        .checked_minus(amount)
                        .and_then(MoneyDelta::to_money)
                        .map_err(|_| CustodyError::AmountInvalid)?;
                    let mut pairs = safe.non_zero();
                    pairs.push((currency, new_money));
                    *safe = Balances::of(&pairs);
                }
                let r = new_state
                    .requests
                    .iter_mut()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                r.stage = Some(TransferStage::InTransit);
                r.dispatched_at = Some(now);
                r.dispatched_by = Some(me.to_string());
                return Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::Dispatch,
                        subject: Some(id.clone()),
                    }],
                    effects,
                    id: None,
                });
            }
            CustodyCommand::Commit { id } => {
                let req = state
                    .requests
                    .iter()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                // 1. Role is Backend, else RoleNotAllowed
                if actor.role != CustodyRole::Backend {
                    return Err(CustodyError::RoleNotAllowed);
                }
                // 2. Status is Completed -> accepted, nothing changes, no event, no effect
                if req.status == RequestStatus::Completed {
                    return Ok(Transition {
                        state: state.clone(),
                        events: Vec::new(),
                        effects: Vec::new(),
                        id: None,
                    });
                }
                // 3. Status is not Pending -> RequestFinished
                if req.status != RequestStatus::Pending {
                    return Err(CustodyError::RequestFinished);
                }
                // 4. I am the request's sender or receiver -> BackendIsParty
                if me == req.sender || req.receiver.as_deref() == Some(me) {
                    return Err(CustodyError::BackendIsParty);
                }
                let has = |role: CustodyRole| {
                    req.signatures
                        .iter()
                        .any(|s| s.role == role && s.employee_id != req.sender)
                };
                // 5. Kind Handover or Close and not has(Manager) -> AwaitingManager
                if matches!(req.kind, RequestKind::Handover | RequestKind::Close)
                    && !has(CustodyRole::Manager)
                {
                    return Err(CustodyError::AwaitingManager);
                }
                // 6. Kind Handover, Close or SafeOpen and not has(Monitor) -> AwaitingMonitor
                if matches!(
                    req.kind,
                    RequestKind::Handover | RequestKind::Close | RequestKind::SafeOpen
                ) && !has(CustodyRole::Monitor)
                {
                    return Err(CustodyError::AwaitingMonitor);
                }
                // 7. Kind Handover and no signature with employee_id == receiver and role Front -> AwaitingReceiver
                if req.kind == RequestKind::Handover {
                    let rcv = req.receiver.as_deref().unwrap_or_default();
                    let receiver_signed = req
                        .signatures
                        .iter()
                        .any(|s| s.employee_id == rcv && s.role == CustodyRole::Front);
                    if !receiver_signed {
                        return Err(CustodyError::AwaitingReceiver);
                    }
                }
                // 8. Kind is a transfer and (no signature with employee_id == receiver, or not has(Finance)) -> AwaitingFinance
                if req.kind.is_transfer() {
                    let rcv = req.receiver.as_deref().unwrap_or_default();
                    let receiver_signed = req.signatures.iter().any(|s| s.employee_id == rcv);
                    if !receiver_signed || !has(CustodyRole::Finance) {
                        return Err(CustodyError::AwaitingFinance);
                    }
                }
                // 9. The request's shift_id is Some and it is not the open shift's id (or there is no open shift) -> ShiftChanged
                if let Some(req_shift_id) = &req.shift_id {
                    if facts.open_shift.as_ref().map(|s| &s.id) != Some(req_shift_id) {
                        return Err(CustodyError::ShiftChanged);
                    }
                }
                // 10. By kind
                let mut new_state = state.clone();
                let mut effects = Vec::new();
                match req.kind {
                    RequestKind::Open => {
                        if facts.open_shift.is_some() {
                            return Err(CustodyError::ShiftAlreadyOpen);
                        }
                        let counted = req
                            .counted
                            .as_ref()
                            .ok_or(CustodyError::DenominationsRequired)?;
                        effects.push(ShiftEffect::OpenShift {
                            request_id: Some(req.request_id.clone()),
                            opening: counted.totals.clone(),
                        });
                        new_state.custodian = Some(req.sender.clone());
                        new_state.phase = Phase::Ready;
                    }
                    RequestKind::SafeOpen => {
                        if state.safe.is_some() {
                            return Err(CustodyError::SafeAlreadyOpened);
                        }
                        let counted = req
                            .counted
                            .as_ref()
                            .ok_or(CustodyError::DenominationsRequired)?;
                        new_state.safe = Some(counted.totals.clone());
                    }
                    RequestKind::Handover | RequestKind::Close => {
                        let open_shift = facts
                            .open_shift
                            .as_ref()
                            .ok_or(CustodyError::BalancesChanged)?;
                        if Some(&open_shift.expected) != req.expected.as_ref()
                            || state.safe.as_ref() != req.safe_expected.as_ref()
                        {
                            return Err(CustodyError::BalancesChanged);
                        }
                        let counted = req
                            .counted
                            .as_ref()
                            .ok_or(CustodyError::DenominationsRequired)?;
                        let safe_count = req
                            .safe_count
                            .as_ref()
                            .ok_or(CustodyError::DenominationsRequired)?;
                        let shift_id = req.shift_id.clone().ok_or(CustodyError::BalancesChanged)?;
                        let expected = req.expected.clone().ok_or(CustodyError::BalancesChanged)?;
                        effects.push(ShiftEffect::CloseShift {
                            request_id: req.request_id.clone(),
                            shift_id,
                            expected,
                            counted: counted.totals.clone(),
                            note: req.note.clone(),
                        });
                        new_state.safe = Some(safe_count.totals.clone());
                        if req.kind == RequestKind::Handover {
                            effects.push(ShiftEffect::OpenShift {
                                request_id: None,
                                opening: counted.totals.clone(),
                            });
                            new_state.custodian = req.receiver.clone();
                            new_state.phase = Phase::Open;
                        } else {
                            new_state.custodian = None;
                            new_state.phase = Phase::Closed;
                        }
                    }
                    RequestKind::SafeIn
                    | RequestKind::SafeOut
                    | RequestKind::ExternalIn
                    | RequestKind::ExternalOut => {
                        if req.stage != Some(TransferStage::InTransit) {
                            return Err(CustodyError::DispatchRequired);
                        }
                        let shift_id =
                            req.shift_id.clone().ok_or(CustodyError::DispatchRequired)?;
                        let currency = req.currency.ok_or(CustodyError::DispatchRequired)?;
                        let amount = req.amount.ok_or(CustodyError::DispatchRequired)?;
                        if matches!(req.kind, RequestKind::SafeOut | RequestKind::ExternalIn) {
                            let note_or_id = if req.note.is_empty() {
                                &req.id
                            } else {
                                &req.note
                            };
                            effects.push(ShiftEffect::MoveStock {
                                request_id: None,
                                shift_id,
                                currency,
                                direction: Direction::In,
                                amount,
                                reason: movement_reason(req.kind, note_or_id),
                            });
                        } else if req.kind == RequestKind::SafeIn {
                            let safe =
                                new_state.safe.as_mut().ok_or(CustodyError::SafeNotOpened)?;
                            let curr_money = safe.get(currency);
                            let new_money = MoneyDelta::from(curr_money)
                                .checked_plus(amount)
                                .and_then(MoneyDelta::to_money)
                                .map_err(|_| CustodyError::AmountInvalid)?;
                            let mut pairs = safe.non_zero();
                            pairs.push((currency, new_money));
                            *safe = Balances::of(&pairs);
                        }
                    }
                }
                // 11. Status = Completed, completed_at, Signature, reconciliation
                let r = new_state
                    .requests
                    .iter_mut()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                r.status = RequestStatus::Completed;
                r.completed_at = Some(now);
                r.signatures.push(Signature {
                    employee_id: me.to_string(),
                    role: CustodyRole::Backend,
                    at: now,
                });
                r.reconciliation = Some(Reconciliation::Pending);
                if r.kind.is_transfer() {
                    r.stage = Some(TransferStage::Received);
                }
                // 12. CashException if any difference has non-zero entry
                if has_non_zero(&r.target_difference)
                    || has_non_zero(&r.variance)
                    || has_non_zero(&r.safe_variance)
                {
                    new_state.exceptions.push(CashException {
                        id: facts.new_id.clone(),
                        request_id: r.id.clone(),
                        status: ExceptionStatus::Open,
                        target_difference: r.target_difference.clone(),
                        cash_variance: r.variance.clone(),
                        safe_variance: r.safe_variance.clone(),
                        note: r.note.clone(),
                        at: now,
                        reviewed_by: None,
                        reviewed_at: None,
                    });
                }
                // 13. Event Commit
                return Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::Commit,
                        subject: Some(id.clone()),
                    }],
                    effects,
                    id: None,
                });
            }
            CustodyCommand::Reconcile { id, note } => {
                let req = state
                    .requests
                    .iter()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                if actor.role != CustodyRole::Finance {
                    return Err(CustodyError::RoleNotAllowed);
                }
                if me == req.sender || req.receiver.as_deref() == Some(me) {
                    return Err(CustodyError::ReconcilerIsParty);
                }
                if req.status != RequestStatus::Completed {
                    return Err(CustodyError::NotCompleted);
                }
                if note.trim().is_empty() {
                    return Err(CustodyError::ReconcileNoteRequired);
                }
                let mut new_state = state.clone();
                let r = new_state
                    .requests
                    .iter_mut()
                    .find(|r| r.id == *id)
                    .ok_or(CustodyError::RequestNotFound)?;
                r.reconciliation = Some(Reconciliation::Checked);
                r.reconciled = Some(Decision {
                    by: me.to_string(),
                    at: now,
                    text: cut_to(note, 1000).to_string(),
                });
                for exc in &mut new_state.exceptions {
                    if exc.request_id == *id {
                        exc.status = ExceptionStatus::Reviewed;
                        exc.reviewed_by = Some(me.to_string());
                        exc.reviewed_at = Some(now);
                    }
                }
                return Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::Reconcile,
                        subject: Some(id.clone()),
                    }],
                    effects: Vec::new(),
                    id: None,
                });
            }
            _ => {}
        }

        // 8.1.6: Then the command's own checks.
        match cmd {
            CustodyCommand::BeginTransaction {
                side,
                currency,
                amount,
            } => {
                if actor.role != CustodyRole::Front {
                    return Err(CustodyError::RoleNotAllowed);
                }
                if state.custodian.as_deref() != Some(me) || state.phase != Phase::Open {
                    return Err(CustodyError::NotCustodianInOpenService);
                }
                if !facts.service_window.allows_begin(now) {
                    return Err(CustodyError::OutsideServiceHours);
                }
                if currency.is_thb() {
                    return Err(CustodyError::DraftInvalid);
                }
                if amount.is_zero() {
                    return Err(CustodyError::AmountInvalid);
                }
                let open_shift = facts.open_shift.as_ref().ok_or(CustodyError::NoOpenShift)?;
                let mut new_state = state.clone();
                new_state.drafts.push(TxDraft {
                    id: facts.new_id.clone(),
                    shift_id: open_shift.id.clone(),
                    employee_id: me.to_string(),
                    side: *side,
                    currency: *currency,
                    amount: *amount,
                    at: now,
                    status: DraftStatus::Open,
                    document_ref: None,
                });
                Ok(Transition {
                    state: new_state,
                    events: Vec::new(),
                    effects: Vec::new(),
                    id: Some(facts.new_id.clone()),
                })
            }
            CustodyCommand::SetTarget { amounts } => {
                if actor.role != CustodyRole::Finance {
                    return Err(CustodyError::RoleNotAllowed);
                }
                let mut new_state = state.clone();
                new_state.target = Some(CashTarget {
                    amounts: amounts.clone(),
                    by: me.to_string(),
                    at: now,
                });
                Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::CashTarget,
                        subject: None,
                    }],
                    effects: Vec::new(),
                    id: None,
                })
            }
            CustodyCommand::StartService {
                equipment_ready,
                rates_verified,
                rate_updated,
            } => {
                if actor.role != CustodyRole::Backend {
                    return Err(CustodyError::RoleNotAllowed);
                }
                if facts.open_shift.is_none()
                    || state.phase != Phase::Ready
                    || !facts.service_window.allows_start(now)
                {
                    return Err(CustodyError::ServiceNotReady);
                }
                if !*equipment_ready || !*rates_verified || rate_updated != &facts.rates_updated {
                    return Err(CustodyError::ServiceChecksIncomplete);
                }
                let mut new_state = state.clone();
                new_state.phase = Phase::Open;
                new_state.service_opened_at = Some(now);
                Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::ServiceOpen,
                        subject: None,
                    }],
                    effects: Vec::new(),
                    id: None,
                })
            }
            CustodyCommand::Cutoff => {
                if actor.role != CustodyRole::Backend {
                    return Err(CustodyError::RoleNotAllowed);
                }
                let open_shift = facts.open_shift.as_ref().ok_or(CustodyError::NoOpenShift)?;
                if state.phase == Phase::Counting {
                    return Err(CustodyError::CountingUnderWay);
                }
                let mut new_state = state.clone();
                new_state.phase = Phase::CutOff;
                new_state.cutoff = Some(Cutoff {
                    at: now,
                    by: me.to_string(),
                    shift_id: open_shift.id.clone(),
                    last_ref: facts.last_transaction.clone(),
                    pending: facts.pending_transactions.clone(),
                });
                Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::CutOff,
                        subject: None,
                    }],
                    effects: Vec::new(),
                    id: None,
                })
            }
            CustodyCommand::Hold {
                customer_id,
                reason,
            } => {
                if !matches!(actor.role, CustodyRole::Front | CustodyRole::Monitor) {
                    return Err(CustodyError::RoleNotAllowed);
                }
                if !facts.customer_known || reason.trim().is_empty() {
                    return Err(CustodyError::HoldInvalid);
                }
                let mut new_state = state.clone();
                new_state.holds.push(CustomerHold {
                    id: facts.new_id.clone(),
                    customer_id: customer_id.clone(),
                    reason: cut_to(reason, 500).to_string(),
                    by: me.to_string(),
                    at: now,
                    status: HoldStatus::Open,
                    reviewed_by: None,
                    reviewed_at: None,
                    note: None,
                });
                Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::HoldCustomer,
                        subject: Some(customer_id.clone()),
                    }],
                    effects: Vec::new(),
                    id: None,
                })
            }
            CustodyCommand::Release { hold_id, note } => {
                if actor.role != CustodyRole::Monitor {
                    return Err(CustodyError::RoleNotAllowed);
                }
                let hold = state.holds.iter().find(|h| h.id == *hold_id);
                match hold {
                    Some(h) if h.status == HoldStatus::Open && !note.trim().is_empty() => {}
                    _ => return Err(CustodyError::ReleaseInvalid),
                }
                let mut new_state = state.clone();
                let h = new_state
                    .holds
                    .iter_mut()
                    .find(|h| h.id == *hold_id)
                    .ok_or(CustodyError::ReleaseInvalid)?;
                h.status = HoldStatus::Released;
                h.reviewed_by = Some(me.to_string());
                h.reviewed_at = Some(now);
                h.note = Some(cut_to(note, 500).to_string());
                Ok(Transition {
                    state: new_state,
                    events: vec![CustodyEvent {
                        kind: EventKind::ReleaseCustomer,
                        subject: Some(hold_id.clone()),
                    }],
                    effects: Vec::new(),
                    id: None,
                })
            }
            CustodyCommand::Propose(p) => Self::apply_propose(state, p, actor, me, facts, now),
            CustodyCommand::SetEntity { .. }
            | CustodyCommand::Enable
            | CustodyCommand::Clock { .. }
            | CustodyCommand::Reject { .. }
            | CustodyCommand::Sign { .. }
            | CustodyCommand::Dispatch { .. }
            | CustodyCommand::Commit { .. }
            | CustodyCommand::Reconcile { .. } => Err(CustodyError::RoleNotAllowed),
        }
    }

    fn apply_propose(
        state: &CustodyState,
        p: &Proposal,
        actor: &CustodyActor,
        me: &str,
        facts: &CustodyFacts,
        now: CustodyTime,
    ) -> Result<Transition, CustodyError> {
        let kind = p.kind;
        // 1. Role is Front or Finance, else RoleNotAllowed
        if !matches!(actor.role, CustodyRole::Front | CustodyRole::Finance) {
            return Err(CustodyError::RoleNotAllowed);
        }
        // 2. Repeat
        if let Some(existing) = state.requests.iter().find(|r| r.request_id == p.request_id) {
            if existing.payload_hash != p.payload_hash {
                return Err(CustodyError::RequestChanged);
            } else {
                return Ok(Transition {
                    state: state.clone(),
                    events: Vec::new(),
                    effects: Vec::new(),
                    id: Some(existing.id.clone()),
                });
            }
        }
        // 3. request_id does not look like an id -> RequestIdInvalid
        if !looks_like_an_id(&p.request_id) {
            return Err(CustodyError::RequestIdInvalid);
        }
        // 4. evidence_keys empty, > 5, or entry does not look like an id -> EvidenceRequired
        if p.evidence_keys.is_empty()
            || p.evidence_keys.len() > 5
            || p.evidence_keys.iter().any(|k| !looks_like_an_id(k))
        {
            return Err(CustodyError::EvidenceRequired);
        }
        // 5. Role per kind
        if kind == RequestKind::SafeOpen {
            if actor.role != CustodyRole::Finance {
                return Err(CustodyError::RoleNotAllowed);
            }
            if state.safe.is_some() {
                return Err(CustodyError::SafeAlreadyOpened);
            }
        } else {
            if actor.role != CustodyRole::Front {
                return Err(CustodyError::RoleNotAllowed);
            }
            if kind != RequestKind::Open && state.custodian.as_deref() != Some(me) {
                return Err(CustodyError::NotDrawerCustodian);
            }
        }
        // 6. Any request has status Pending -> RequestPending
        if state
            .requests
            .iter()
            .any(|r| r.status == RequestStatus::Pending)
        {
            return Err(CustodyError::RequestPending);
        }
        // 7. note is p.note.trim(). length > 500 -> NoteTooLong
        let note_trimmed = p.note.trim();
        if matches!(kind, RequestKind::SafeOut | RequestKind::ExternalIn)
            && utf16_len(&movement_reason(kind, note_trimmed)) > 200
        {
            return Err(CustodyError::TextInvalid);
        }
        if utf16_len(note_trimmed) > 500 {
            return Err(CustodyError::NoteTooLong);
        }

        let mut stored_counted = None;
        let mut stored_target = None;
        let mut stored_target_difference = None;
        let mut stored_expected = None;
        let mut stored_variance = None;
        let mut stored_cutoff = None;
        let mut stored_safe_count = None;
        let mut stored_safe_expected = None;
        let mut stored_safe_variance = None;
        let mut stored_currency = None;
        let mut stored_amount = None;

        // 8. Kinds Open, Handover, Close, SafeOpen: check count sheet p.counted
        if matches!(
            kind,
            RequestKind::Open | RequestKind::Handover | RequestKind::Close | RequestKind::SafeOpen
        ) {
            validate_count_sheet(&p.counted)?;
            stored_counted = Some(p.counted.clone());
        }

        // 9. Kind Open
        if kind == RequestKind::Open {
            if !facts.has_live_rate {
                return Err(CustodyError::NoLiveRate);
            }
            if facts.open_shift.is_some() {
                return Err(CustodyError::ShiftAlreadyOpen);
            }
            let target = state.target.as_ref().ok_or(CustodyError::TargetMissing)?;
            stored_target_difference =
                Some(ShiftLedger::difference(&target.amounts, &p.counted.totals));
            stored_target = Some(target.clone());
        }

        // 10. Kinds Handover, Close
        if matches!(kind, RequestKind::Handover | RequestKind::Close) {
            if state.phase != Phase::CutOff {
                return Err(CustodyError::CutoffRequired);
            }
            if !facts.pending_transactions.is_empty() {
                return Err(CustodyError::TransactionsPending);
            }
            let open_shift = facts.open_shift.as_ref().ok_or(CustodyError::NoOpenShift)?;
            stored_expected = Some(open_shift.expected.clone());
            stored_variance = Some(ShiftLedger::difference(
                &open_shift.expected,
                &p.counted.totals,
            ));
            stored_cutoff = state.cutoff.clone();
            validate_count_sheet(&p.safe_count)?;
            stored_safe_count = Some(p.safe_count.clone());
            let safe = state.safe.as_ref().ok_or(CustodyError::SafeNotOpened)?;
            stored_safe_expected = Some(safe.clone());
            stored_safe_variance = Some(ShiftLedger::difference(safe, &p.safe_count.totals));
            if kind == RequestKind::Handover {
                match &p.receiver {
                    Some(rcv) if !rcv.is_empty() && rcv != me => {}
                    _ => return Err(CustodyError::HandoverReceiverInvalid),
                }
            }
        }

        // 11. The four transfers
        if kind.is_transfer() {
            if state.phase == Phase::Counting {
                return Err(CustodyError::ClosingCountUnderWay);
            }
            let amount = p.amount.ok_or(CustodyError::AmountMissing)?;
            if amount.is_zero() {
                return Err(CustodyError::AmountInvalid);
            }
            let the_seven = &CurrencyCode::all()[..7];
            let curr = match p.currency {
                Some(c) if the_seven.contains(&c) => c,
                _ => return Err(CustodyError::TransferPartiesInvalid),
            };
            match &p.receiver {
                Some(rcv) if !rcv.is_empty() && rcv != me => {}
                _ => return Err(CustodyError::TransferPartiesInvalid),
            }
            let open_shift = facts.open_shift.as_ref().ok_or(CustodyError::NoOpenShift)?;
            stored_expected = Some(open_shift.expected.clone());
            stored_currency = Some(curr);
            stored_amount = Some(amount);
            if matches!(kind, RequestKind::SafeIn | RequestKind::SafeOut) && state.safe.is_none() {
                return Err(CustodyError::SafeNotOpened);
            }
        }

        // 12. Variance needs note
        if (has_non_zero(&stored_target_difference)
            || has_non_zero(&stored_variance)
            || has_non_zero(&stored_safe_variance))
            && note_trimmed.is_empty()
        {
            return Err(CustodyError::VarianceNeedsNote);
        }

        // 13. Accepted
        let mut new_state = state.clone();
        if matches!(kind, RequestKind::Handover | RequestKind::Close) {
            new_state.phase = Phase::Counting;
        }
        let receiver = match &p.receiver {
            Some(r) if !r.is_empty() => Some(r.clone()),
            _ => None,
        };
        let req = CustodyRequest {
            id: facts.new_id.clone(),
            request_id: p.request_id.clone(),
            payload_hash: p.payload_hash.clone(),
            kind,
            status: RequestStatus::Pending,
            at: now,
            sender: me.to_string(),
            receiver,
            note: note_trimmed.to_string(),
            evidence_keys: p.evidence_keys.clone(),
            signatures: vec![Signature {
                employee_id: me.to_string(),
                role: actor.role,
                at: now,
            }],
            shift_id: facts.open_shift.as_ref().map(|s| s.id.clone()),
            counted: stored_counted,
            target: stored_target,
            target_difference: stored_target_difference,
            expected: stored_expected,
            variance: stored_variance,
            cutoff: stored_cutoff,
            safe_count: stored_safe_count,
            safe_expected: stored_safe_expected,
            safe_variance: stored_safe_variance,
            currency: stored_currency,
            amount: stored_amount,
            stage: None,
            dispatched_at: None,
            dispatched_by: None,
            rejected: None,
            completed_at: None,
            reconciliation: None,
            reconciled: None,
        };
        new_state.requests.push(req);

        Ok(Transition {
            state: new_state,
            events: vec![CustodyEvent {
                kind: EventKind::Propose,
                subject: Some(facts.new_id.clone()),
            }],
            effects: Vec::new(),
            id: Some(facts.new_id.clone()),
        })
    }

    pub fn check_save(
        state: &CustodyState,
        save: &SaveAttempt,
        actor: &CustodyActor,
        facts: &CustodyFacts,
        now: CustodyTime,
    ) -> Result<(), CustodyError> {
        // 1. state.enabled is false -> Ok(()).
        if !state.enabled {
            return Ok(());
        }

        // 2. save.prior:
        match save.prior {
            PriorSave::SavedByActor => {
                if actor.role == CustodyRole::Front {
                    return Ok(());
                } else {
                    return Err(CustodyError::SaveNotOwnRetry);
                }
            }
            PriorSave::SavedByOther => {
                return Err(CustodyError::SaveNotOwnRetry);
            }
            PriorSave::None | PriorSave::Unfinished => {}
        }

        // 3. A hold with customer_id == save.customer_id and status Open exists -> CustomerOnHold.
        if state
            .holds
            .iter()
            .any(|h| h.customer_id == save.customer_id && h.status == HoldStatus::Open)
        {
            return Err(CustodyError::CustomerOnHold);
        }

        // 4. Only when save.prior == PriorSave::None:
        let me = actor.employee_id.as_deref().unwrap_or_default();
        if save.prior == PriorSave::None {
            if !facts.service_window.allows_save(now) {
                return Err(CustodyError::SaveClosed);
            }
            if state.phase != Phase::Open && state.phase != Phase::CutOff {
                return Err(CustodyError::SaveClosed);
            }
            let needs_draft = state.phase != Phase::Open || facts.service_window.needs_draft(now);
            if needs_draft {
                let open_shift_id = facts.open_shift.as_ref().map(|s| s.id.as_str());
                let has_matching_draft = state.drafts.iter().any(|d| {
                    draft_matches_save(d, save)
                        && d.employee_id == me
                        && open_shift_id == Some(d.shift_id.as_str())
                });
                if !has_matching_draft {
                    return Err(CustodyError::SaveClosed);
                }
            }
        }

        // 5. My role is not Front, or I have no employee_id, or I am not state.custodian, or I am not clocked in -> SaveNotCustodian.
        if actor.role != CustodyRole::Front
            || actor.employee_id.is_none()
            || state.custodian.as_deref() != Some(me)
            || !is_clocked_in(state, me)
        {
            return Err(CustodyError::SaveNotCustodian);
        }

        // 6. Ok(()).
        Ok(())
    }

    pub fn complete_draft(
        state: &CustodyState,
        save: &SaveAttempt,
        document_ref: &str,
    ) -> CustodyState {
        // One exchange completes at most one draft: its number already
        // finished one, so this repeat finishes no other (ticket #154).
        if state
            .drafts
            .iter()
            .any(|d| d.document_ref.as_deref() == Some(document_ref))
        {
            return state.clone();
        }
        let mut new_state = state.clone();
        for draft in &mut new_state.drafts {
            if draft_matches_save(draft, save) {
                draft.status = DraftStatus::Completed;
                draft.document_ref = Some(document_ref.to_string());
                break;
            }
        }
        new_state
    }
}
