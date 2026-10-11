//! The steps other contexts add around the save of an exchange.
//!
//! Saving an exchange is `SaveTransactionWorkflow`'s; custody, the devices and
//! the kiosk each have something to check before it and something to record
//! after it. Each of them is one `SaveStep` in a file of its own, and the
//! workflow runs the registered steps inside the same branch transaction as the
//! save: a step that refuses, or fails, undoes the whole save, and what a step
//! writes exists exactly when the exchange does.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_kernel::ids::BranchId;

/// Who saves.
///
/// `id` is the identity every other context is given (the owner of the files,
/// `recorded_by`, the custody and kiosk actor). `username` is only what the
/// form's `staff` field is set to and what is printed on the receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveActor {
    pub id: String,
    pub username: String,
    /// The employee's id for a staff member; `None` for the owner.
    pub employee_id: Option<String>,
    pub role: Role,
}

/// What the screens send with a save that is not part of the exchange itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SaveExtras {
    /// The custody draft a Front began the exchange under.
    pub staff_draft_id: Option<String>,
    /// The cash was counted before saving.
    pub count_confirmed: bool,
    /// The kiosk request this exchange completes.
    pub kiosk_request_id: Option<String>,
}

/// A booking claimed by the save of an exchange (T15-h).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookingClaim {
    pub booking_id: String,
    pub customer_confirmed: bool,
}

/// What a step is told about the save it takes part in.
#[derive(Debug, Clone, Copy)]
pub struct SaveContext<'a> {
    pub branch: &'a BranchId,
    /// The parsed form; its `staff` is already the saver's username.
    pub form: &'a TransactionForm,
    pub extras: &'a SaveExtras,
    pub booking: Option<&'a BookingClaim>,
    pub actor: &'a SaveActor,
    /// The Blueprint-session teller saving (CA-7 #623, `SaveTransactionWorkflow::save_as_teller`): the exchange
    /// posts to their own OPEN shift. `None` for every other save, which keeps v2's path until CA-10 #626.
    pub teller: Option<&'a xc_auth::domain::access::Actor>,
    /// The time the save was given, cut to milliseconds.
    pub now: DateTime<Utc>,
    /// The exchange already saved under this submission in this branch and
    /// mode, when the save is a repeat; `None` for a first save.
    pub prior: Option<&'a Transaction>,
}

/// The future of a step.
pub type StepFuture<'a> = Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>>;

/// One context's part in the save of an exchange.
pub trait SaveStep: Send + Sync + 'static {
    /// Whether this step vouches for the form's rate, skipping the
    /// counter-rate check.
    fn vouches_for_rate(&self, _ctx: &SaveContext<'_>) -> bool {
        false
    }

    /// Runs inside the save's transaction after the form is judged and before
    /// anything of the exchange is checked or written. An error ends the save
    /// with that error.
    fn before_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
    ) -> StepFuture<'a>;

    /// Runs inside the save's transaction once the exchange is stored — and
    /// again each time the same submission is answered from the stored one, so
    /// it must be safe to repeat. An error undoes the save.
    fn after_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
        saved: &'a Transaction,
    ) -> StepFuture<'a>;
}
