//! A print or drawer job and the two things it is described by.

use chrono::{DateTime, Utc};

/// What a job asks a device to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    PrintReceipt,
    DrawerRequest,
}

impl JobKind {
    /// The kind a screen names, or `None` for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "PRINT_RECEIPT" => Some(Self::PrintReceipt),
            "DRAWER_REQUEST" => Some(Self::DrawerRequest),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrintReceipt => "PRINT_RECEIPT",
            Self::DrawerRequest => "DRAWER_REQUEST",
        }
    }
}

/// Where a job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Failed,
    SimulatedPrinted,
    CommandSimulated,
}

impl JobStatus {
    /// The status a row holds, or `None` for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "QUEUED" => Some(Self::Queued),
            "FAILED" => Some(Self::Failed),
            "SIMULATED_PRINTED" => Some(Self::SimulatedPrinted),
            "COMMAND_SIMULATED" => Some(Self::CommandSimulated),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Failed => "FAILED",
            Self::SimulatedPrinted => "SIMULATED_PRINTED",
            Self::CommandSimulated => "COMMAND_SIMULATED",
        }
    }
}

/// One job of one branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceJob {
    pub id: String,
    pub request_id: String,
    pub branch_id: String,
    /// The document number of the exchange the job is about; empty when none.
    pub reference: String,
    pub kind: JobKind,
    pub status: JobStatus,
    /// Who queued it.
    pub actor: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
