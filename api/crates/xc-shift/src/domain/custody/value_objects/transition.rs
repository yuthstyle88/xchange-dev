//! Transition result of applying a custody command.

use crate::domain::custody::state::CustodyState;
use crate::domain::custody::value_objects::custody_event::CustodyEvent;
use crate::domain::custody::value_objects::shift_effect::ShiftEffect;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub state: CustodyState,
    pub events: Vec<CustodyEvent>,
    pub effects: Vec<ShiftEffect>,
    pub id: Option<String>,
}
