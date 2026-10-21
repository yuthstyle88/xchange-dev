//! Approval request domain model and outcome.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::fmt::{self, Display, Formatter};

/// The outcome of an approval request decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Approved,
    Rejected,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "APPROVED",
            Self::Rejected => "REJECTED",
        }
    }
}

impl Display for Outcome {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The unique identifier of an approval request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(String);

impl RequestId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for RequestId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for RequestId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for RequestId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// An approval request record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub id: RequestId,
    pub organization_id: String,
    pub type_name: String,
    pub branch_id: Option<String>,
    pub object_id: String,
    pub object_version_hash: String,
    pub policy_version: String,
    pub proposed_payload: Value,
    pub requested_by: String,
    pub requested_at: DateTime<Utc>,
    pub status: String,
    pub idempotency_key: String,
}
