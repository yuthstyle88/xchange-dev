//! `ApprovalPolicy`: the per-type policy trait and refusal error.

use xc_auth::domain::access::Actor;

use crate::domain::request::ApprovalRequest;

/// A refusal returned by an `ApprovalPolicy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    code: &'static str,
    message: String,
}

impl Refusal {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

/// The policy governing decisions for an approval subject.
pub trait ApprovalPolicy: Send + Sync {
    /// Checks whether `who` may decide `request`.
    ///
    /// The core has already verified that `who` is not the proposer.
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal>;

    /// Whether the proposer `who` may close their own request at submit (A5).
    ///
    /// Defaults to `false`.
    fn allows_self_decision(&self, who: &Actor) -> bool {
        let _ = who;
        false
    }

    /// The version string of this policy.
    fn policy_version(&self) -> String;
}
