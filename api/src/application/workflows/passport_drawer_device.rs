//! The passport drawer: the seam the desk's passport-drawer request goes through (design §5.5.8).
//! Row S0 lands the seam; row AG-PD writes the device behind it and row PD-0 calls it.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::infrastructure::devices::drawer_job::PassportSource;

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

/// The outcome of asking the passport drawer to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassportDrawerOutcome {
    /// Rows 11 and 12 (and NoAgent / NoDrawer): `duplicate` true only for row 11.
    State {
        state: PassportDrawerState,
        duplicate: bool,
    },
    /// Row 9.
    NotHolder,
    /// Row 10.
    OtherEmployee,
}

/// Asks the branch's passport drawer to open for one order.
pub trait PassportDrawerDevice: Send + Sync {
    /// Row 7: whether `devices.passportDrawerBranches` lists the branch.
    fn switched_on(&self, branch: &BranchId) -> bool;
    /// Rows 9-12 for an order id that passed row 8.
    fn request<'a>(
        &'a self,
        branch: &'a BranchId,
        source: PassportSource,
        order_id: &'a str,
        actor: &'a str,
        now: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = AppResult<PassportDrawerOutcome>> + Send + 'a>>;
}
