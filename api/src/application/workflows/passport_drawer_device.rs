//! The passport drawer: the seam the desk's passport-drawer request goes through (design §5.5.8).
//! Row S0 lands the seam; row AG-PD writes the device behind it and row PD-0 calls it.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use xc_kernel::ids::BranchId;

/// What became of one passport-drawer request, in v2's words (`desk-passport-drawer.js:7` of the
/// 10-06 bundle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassportDrawerState {
    Simulated,
    Disabled,
    Requested,
    SubmittedToCups,
    Uncertain,
}

impl PassportDrawerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Simulated => "SIMULATED",
            Self::Disabled => "DISABLED",
            Self::Requested => "REQUESTED",
            Self::SubmittedToCups => "SUBMITTED_TO_CUPS",
            Self::Uncertain => "UNCERTAIN",
        }
    }
}

/// Asks the branch's passport drawer to open for one order.
pub trait PassportDrawerDevice: Send + Sync {
    fn request<'a>(
        &'a self,
        branch: &'a BranchId,
        order_key: &'a str,
        actor: &'a str,
        now: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = PassportDrawerState> + Send + 'a>>;
}
