//! Events emitted by the custody machine.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    SetEntity,
    Enable,
    ClockIn,
    ClockOut,
    CashTarget,
    ServiceOpen,
    CutOff,
    Propose,
    HoldCustomer,
    ReleaseCustomer,
    Reject,
    Sign,
    Dispatch,
    Commit,
    Reconcile,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::SetEntity => "SET_ENTITY",
            EventKind::Enable => "ENABLE",
            EventKind::ClockIn => "CLOCK_IN",
            EventKind::ClockOut => "CLOCK_OUT",
            EventKind::CashTarget => "CASH_TARGET",
            EventKind::ServiceOpen => "SERVICE_OPEN",
            EventKind::CutOff => "CUT_OFF",
            EventKind::Propose => "PROPOSE",
            EventKind::HoldCustomer => "HOLD_CUSTOMER",
            EventKind::ReleaseCustomer => "RELEASE_CUSTOMER",
            EventKind::Reject => "REJECT",
            EventKind::Sign => "SIGN",
            EventKind::Dispatch => "DISPATCH",
            EventKind::Commit => "COMMIT",
            EventKind::Reconcile => "RECONCILE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyEvent {
    pub kind: EventKind,
    pub subject: Option<String>,
}
