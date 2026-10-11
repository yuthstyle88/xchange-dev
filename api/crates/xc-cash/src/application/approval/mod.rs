//! Approval integrations for cash transfers.

pub mod transfer;

pub use transfer::{TransferApprovalPolicy, TransferApprovalSubject, policy_actor};
