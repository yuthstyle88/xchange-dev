//! `CaseStatus` (design V2, §4.2): exactly the Blueprint's eight states.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaseStatus {
    Draft,
    Submitted,
    UnderReview,
    Verified,
    Rejected,
    NeedsUpdate,
    Expired,
    Revoked,
}

impl CaseStatus {
    pub const ALL: [CaseStatus; 8] = [
        Self::Draft,
        Self::Submitted,
        Self::UnderReview,
        Self::Verified,
        Self::Rejected,
        Self::NeedsUpdate,
        Self::Expired,
        Self::Revoked,
    ];

    /// The `verification_cases.status` text (`DRAFT`, `SUBMITTED`, …).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::Submitted => "SUBMITTED",
            Self::UnderReview => "UNDER_REVIEW",
            Self::Verified => "VERIFIED",
            Self::Rejected => "REJECTED",
            Self::NeedsUpdate => "NEEDS_UPDATE",
            Self::Expired => "EXPIRED",
            Self::Revoked => "REVOKED",
        }
    }

    /// The status of a stored text; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "DRAFT" => Some(Self::Draft),
            "SUBMITTED" => Some(Self::Submitted),
            "UNDER_REVIEW" => Some(Self::UnderReview),
            "VERIFIED" => Some(Self::Verified),
            "REJECTED" => Some(Self::Rejected),
            "NEEDS_UPDATE" => Some(Self::NeedsUpdate),
            "EXPIRED" => Some(Self::Expired),
            "REVOKED" => Some(Self::Revoked),
            _ => None,
        }
    }

    /// `DRAFT`, `SUBMITTED`, `UNDER_REVIEW`, `NEEDS_UPDATE`: the states `verification_one_open_case` counts (V14).
    pub fn is_open(&self) -> bool {
        matches!(
            self,
            Self::Draft | Self::Submitted | Self::UnderReview | Self::NeedsUpdate
        )
    }

    /// `REJECTED`, `EXPIRED`, `REVOKED`: no transition leaves them (V2).
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Rejected | Self::Expired | Self::Revoked)
    }
}
