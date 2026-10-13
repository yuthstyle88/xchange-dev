//! `DecisionReason` (design §12 S-5, decided by the owner 2026-10-12): the only values of
//! `verification_cases.decision_reason_code`. No free text is stored.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionReason {
    DocumentUnreadable,
    DocumentExpired,
    DetailsMismatch,
    DocumentNotAccepted,
    SuspectedFraud,
    Other,
}

impl DecisionReason {
    pub const ALL: [DecisionReason; 6] = [
        Self::DocumentUnreadable,
        Self::DocumentExpired,
        Self::DetailsMismatch,
        Self::DocumentNotAccepted,
        Self::SuspectedFraud,
        Self::Other,
    ];

    /// The stored code: `DOCUMENT_UNREADABLE`, `DOCUMENT_EXPIRED`, `DETAILS_MISMATCH`, `DOCUMENT_NOT_ACCEPTED`,
    /// `SUSPECTED_FRAUD`, `OTHER`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DocumentUnreadable => "DOCUMENT_UNREADABLE",
            Self::DocumentExpired => "DOCUMENT_EXPIRED",
            Self::DetailsMismatch => "DETAILS_MISMATCH",
            Self::DocumentNotAccepted => "DOCUMENT_NOT_ACCEPTED",
            Self::SuspectedFraud => "SUSPECTED_FRAUD",
            Self::Other => "OTHER",
        }
    }

    /// One of the six codes, exactly as written; `None` for anything else (other case, spaces, free text).
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "DOCUMENT_UNREADABLE" => Some(Self::DocumentUnreadable),
            "DOCUMENT_EXPIRED" => Some(Self::DocumentExpired),
            "DETAILS_MISMATCH" => Some(Self::DetailsMismatch),
            "DOCUMENT_NOT_ACCEPTED" => Some(Self::DocumentNotAccepted),
            "SUSPECTED_FRAUD" => Some(Self::SuspectedFraud),
            "OTHER" => Some(Self::Other),
            _ => None,
        }
    }
}
