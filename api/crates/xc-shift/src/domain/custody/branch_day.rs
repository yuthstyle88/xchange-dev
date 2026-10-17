//! The branch day (CA-7 #623, design `cash-shift-transfer.md` §4.2): start service, cut-off, the day's phase and
//! the save rule, on the Blueprint's roles through `Authorizer::check`.
//!
//! The signatures are [required] by `tests/branch_day_test.rs`. The v2 half the custody machine still answers
//! until CA-10 #626 is at the end of this file.

use chrono::{DateTime, Utc};
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};

use crate::domain::custody::entities::custody_machine::is_clocked_in;
use crate::domain::custody::entities::customer_hold::HoldStatus;
use crate::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use crate::domain::custody::state::CustodyState;
use crate::domain::custody::value_objects::custody_actor::CustodyActor;
use crate::domain::custody::value_objects::custody_error::CustodyError;
use crate::domain::custody::value_objects::custody_facts::CustodyFacts;
use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::phase::Phase;
use crate::domain::custody::value_objects::save_attempt::{PriorSave, SaveAttempt};
use crate::domain::custody::value_objects::service_window::ServiceWindow;

/// Stateless: every rule of the branch day, with no I/O.
pub struct BranchDay;

impl BranchDay {
    /// Start service: `BranchDayOperate` on the branch first (BRANCH_MANAGER, §3.2), then READY → OPEN. Outside
    /// READY the refusal carries the branch's own hours as given (#755). The window itself is checked by the caller
    /// (`BranchHours`).
    pub fn start_service(
        phase: Phase,
        actor: &Actor,
        at: &Target,
        now: DateTime<Utc>,
        window: ServiceWindow,
        utc_offset_minutes: i32,
    ) -> Result<Phase, CustodyError> {
        Self::may_operate(actor, at, now)?;
        if phase != Phase::Ready {
            return Err(CustodyError::ServiceNotReady {
                window,
                utc_offset_minutes,
            });
        }
        Ok(Phase::Open)
    }

    /// Cut-off: the same access check, then OPEN → CUT_OFF; any other phase is `DayNotOpen`.
    pub fn cut_off(
        phase: Phase,
        actor: &Actor,
        at: &Target,
        now: DateTime<Utc>,
    ) -> Result<Phase, CustodyError> {
        Self::may_operate(actor, at, now)?;
        if phase != Phase::Open {
            return Err(CustodyError::DayNotOpen);
        }
        Ok(Phase::CutOff)
    }

    /// The day's phase as read: CUT_OFF with no OPEN shift left is CLOSED (§4.2, the last shift's CLOSE was
    /// verified); every other pair reads as stored.
    pub fn read_phase(stored: Phase, open_shifts: usize) -> Phase {
        if stored == Phase::CutOff && open_shifts == 0 {
            Phase::Closed
        } else {
            stored
        }
    }

    /// CA-7e (#1911, design §4.2): a verified OPEN count moves the branch day to READY when the day reads CLOSED
    /// before the count's shift is opened; any other day is left as stored.
    pub fn ready_on_open_count(stored: Phase, open_shifts_before: usize) -> Option<Phase> {
        if Self::read_phase(stored, open_shifts_before) == Phase::Closed {
            Some(Phase::Ready)
        } else {
            None
        }
    }

    /// The save of a Blueprint-session saver (C1, §4.3). A repeat is answered by who saved first. A first save
    /// needs the saver's own OPEN shift, a day that is OPEN or CUT_OFF, and — after cut-off or inside the window's
    /// draft-only part — an OPEN draft of the saver's own on that shift matching the save (the kept rule). There is
    /// no branch custodian.
    pub fn check_save(
        phase: Phase,
        saver: &Actor,
        own_shift: Option<&str>,
        drafts: &[TxDraft],
        save: &SaveAttempt,
        window_needs_draft: bool,
    ) -> Result<(), CustodyError> {
        match save.prior {
            PriorSave::SavedByActor => return Ok(()),
            PriorSave::SavedByOther => return Err(CustodyError::SaveNotOwnRetry),
            PriorSave::None | PriorSave::Unfinished => {}
        }
        let Some(own_shift) = own_shift else {
            return Err(CustodyError::NoOpenShift);
        };
        if save.prior != PriorSave::None {
            return Ok(());
        }
        if phase != Phase::Open && phase != Phase::CutOff {
            return Err(CustodyError::SaveClosed);
        }
        if phase == Phase::CutOff || window_needs_draft {
            let has_own_draft = drafts.iter().any(|d| {
                draft_matches_save(d, save)
                    && d.employee_id == saver.user_id
                    && d.shift_id == own_shift
            });
            if !has_own_draft {
                return Err(CustodyError::SaveClosed);
            }
        }
        Ok(())
    }

    fn may_operate(actor: &Actor, at: &Target, now: DateTime<Utc>) -> Result<(), CustodyError> {
        Authorizer::check(Some(actor), Permission::BranchDayOperate, at, now)
            .map(|_| ())
            .map_err(|_| CustodyError::RoleNotAllowed)
    }
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

impl BranchDay {
    /// The v2 save rule (`CustodyMachine::check_save`).
    pub(crate) fn v2_check_save(
        state: &CustodyState,
        save: &SaveAttempt,
        actor: &CustodyActor,
        facts: &CustodyFacts,
        now: CustodyTime,
    ) -> Result<(), CustodyError> {
        // 1. save.prior:
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

        // 2. A hold with customer_id == save.customer_id and status Open exists -> CustomerOnHold.
        if state
            .holds
            .iter()
            .any(|h| h.customer_id == save.customer_id && h.status == HoldStatus::Open)
        {
            return Err(CustodyError::CustomerOnHold);
        }

        // 3. Only when save.prior == PriorSave::None:
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

        // 4. My role is not Front, or I have no employee_id, or I hold no OPEN shift, or I am not clocked in -> SaveNotCustodian.
        if actor.role != CustodyRole::Front
            || actor.employee_id.is_none()
            || facts.open_shift.is_none()
            || !is_clocked_in(state, me)
        {
            return Err(CustodyError::SaveNotCustodian);
        }

        // 5. Ok(()).
        Ok(())
    }

    /// The draft a save completes (`CustodyMachine::complete_draft`), for both paths.
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
