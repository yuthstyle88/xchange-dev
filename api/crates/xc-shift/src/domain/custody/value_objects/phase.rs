//! Custody phases.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    Closed,
    Ready,
    Open,
    CutOff,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Closed => "CLOSED",
            Phase::Ready => "READY",
            Phase::Open => "OPEN",
            Phase::CutOff => "CUT_OFF",
        }
    }
}
