//! Custody save and draft completion rules.

use crate::domain::custody::branch_day::BranchDay;
use crate::domain::custody::state::CustodyState;
use crate::domain::custody::value_objects::save_attempt::SaveAttempt;

pub struct CustodyMachine;

impl CustodyMachine {
    pub fn complete_draft(
        state: &CustodyState,
        save: &SaveAttempt,
        document_ref: &str,
    ) -> CustodyState {
        BranchDay::complete_draft(state, save, document_ref)
    }
}
