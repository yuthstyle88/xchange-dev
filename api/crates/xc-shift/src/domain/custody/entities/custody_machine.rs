//! Custody save and draft completion rules.

use crate::domain::custody::branch_day::BranchDay;
use crate::domain::custody::state::CustodyState;
use crate::domain::custody::value_objects::custody_actor::CustodyActor;
use crate::domain::custody::value_objects::custody_error::CustodyError;
use crate::domain::custody::value_objects::custody_facts::CustodyFacts;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::save_attempt::SaveAttempt;

pub struct CustodyMachine;

pub(crate) fn is_clocked_in(state: &CustodyState, employee_id: &str) -> bool {
    state
        .attendance
        .iter()
        .any(|a| a.employee_id == employee_id && a.clock_out.is_none())
}

impl CustodyMachine {
    pub fn check_save(
        state: &CustodyState,
        save: &SaveAttempt,
        actor: &CustodyActor,
        facts: &CustodyFacts,
        now: CustodyTime,
    ) -> Result<(), CustodyError> {
        BranchDay::v2_check_save(state, save, actor, facts, now)
    }

    pub fn complete_draft(
        state: &CustodyState,
        save: &SaveAttempt,
        document_ref: &str,
    ) -> CustodyState {
        BranchDay::complete_draft(state, save, document_ref)
    }
}
