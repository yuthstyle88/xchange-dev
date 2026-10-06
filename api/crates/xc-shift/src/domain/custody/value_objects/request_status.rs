//! Custody request status, transfer stage, and reconciliation status.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestStatus {
    Pending,
    Rejected,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferStage {
    InTransit,
    Received,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reconciliation {
    Pending,
    Checked,
}
