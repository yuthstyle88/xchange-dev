//! Custody request kinds.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    Open,
    Handover,
    Close,
    SafeOpen,
    SafeIn,
    SafeOut,
    ExternalIn,
    ExternalOut,
}

impl RequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RequestKind::Open => "OPEN",
            RequestKind::Handover => "HANDOVER",
            RequestKind::Close => "CLOSE",
            RequestKind::SafeOpen => "SAFE_OPEN",
            RequestKind::SafeIn => "SAFE_IN",
            RequestKind::SafeOut => "SAFE_OUT",
            RequestKind::ExternalIn => "EXTERNAL_IN",
            RequestKind::ExternalOut => "EXTERNAL_OUT",
        }
    }

    pub fn is_transfer(self) -> bool {
        matches!(
            self,
            RequestKind::SafeIn
                | RequestKind::SafeOut
                | RequestKind::ExternalIn
                | RequestKind::ExternalOut
        )
    }
}
