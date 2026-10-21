use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelReason {
    CustomerLeft { note: Option<String> },
    CustomerCancelled,
    CustomerChangedMind { note: Option<String> },
    Duplicate { note: Option<String> },
    Other { note: String },
    DayClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CancelReasonError {
    #[error("กรุณาเลือกเหตุผลการยกเลิก (reason)")]
    ReasonRequired,
    #[error("เหตุผลการยกเลิกไม่ถูกต้อง (reason)")]
    ReasonUnknown,
    #[error("กรุณาระบุรายละเอียดเหตุผล (note)")]
    NoteRequired,
    #[error("รายละเอียดเหตุผลต้องไม่เกิน 200 ตัวอักษร (note)")]
    NoteTooLong,
}

impl CancelReason {
    pub fn parse(code: &str, note: Option<&str>) -> Result<Self, CancelReasonError> {
        if code.is_empty() {
            return Err(CancelReasonError::ReasonRequired);
        }
        let note = note.map(str::trim).filter(|s| !s.is_empty());
        if note.is_some_and(|s| s.chars().count() > 200) {
            return Err(CancelReasonError::NoteTooLong);
        }
        match code {
            "CUSTOMER_LEFT" => Ok(Self::CustomerLeft {
                note: note.map(str::to_owned),
            }),
            "CUSTOMER_CHANGED_MIND" => Ok(Self::CustomerChangedMind {
                note: note.map(str::to_owned),
            }),
            "DUPLICATE" => Ok(Self::Duplicate {
                note: note.map(str::to_owned),
            }),
            "OTHER" => match note {
                Some(n) => Ok(Self::Other { note: n.to_owned() }),
                None => Err(CancelReasonError::NoteRequired),
            },
            _ => Err(CancelReasonError::ReasonUnknown),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CustomerLeft { .. } => "CUSTOMER_LEFT",
            Self::CustomerCancelled => "CUSTOMER_CANCELLED",
            Self::CustomerChangedMind { .. } => "CUSTOMER_CHANGED_MIND",
            Self::Duplicate { .. } => "DUPLICATE",
            Self::Other { .. } => "OTHER",
            Self::DayClosed => "DAY_CLOSED",
        }
    }

    pub fn note(&self) -> Option<&str> {
        match self {
            Self::CustomerLeft { note }
            | Self::CustomerChangedMind { note }
            | Self::Duplicate { note } => note.as_deref(),
            Self::Other { note } => Some(note.as_str()),
            Self::CustomerCancelled | Self::DayClosed => None,
        }
    }

    pub fn is_no_show(&self) -> bool {
        matches!(self, Self::CustomerLeft { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cancellation {
    pub at: DateTime<Utc>,
    pub by: String,
    pub reason: CancelReason,
}
