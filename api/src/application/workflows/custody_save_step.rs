//! Custody's part in the save of an exchange.

use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_kernel::mode::Mode;
use xc_kernel::money::CurrencyCode;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::domain::custody::value_objects::custody_actor::CustodyActor;
use xc_shift::domain::custody::value_objects::save_attempt::{PriorSave, SaveAttempt};

use crate::application::workflows::save_step::{SaveContext, SaveStep, StepFuture};

/// Asks custody whether a live exchange may be saved now by this person, and
/// closes the draft it was begun under.
#[derive(Clone)]
pub struct CustodySaveStep {
    custody: CustodyService,
}

impl CustodySaveStep {
    pub fn new(custody: CustodyService) -> Self {
        Self { custody }
    }
}

impl SaveStep for CustodySaveStep {
    /// Live exchanges only. A repeat is judged by who saved the first time
    /// (`recorded_by` against the saver's id), a first save by the whole rule
    /// of `CustodyMachine::check_save` — or, for a Blueprint-session teller
    /// (CA-7 #623), by `BranchDay::check_save` on their own shift. A currency that is no currency is left
    /// to the save itself, which refuses it at the counter rate.
    fn before_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            if ctx.form.customer.mode != Mode::Live {
                return Ok(());
            }
            let Some(attempt) = attempt_of(ctx) else {
                return Ok(());
            };
            if let Some(teller) = ctx.teller {
                return self
                    .custody
                    .check_teller_save_in(tx, ctx.branch, &attempt, teller, ctx.now)
                    .await;
            }
            let actor = CustodyActor {
                id: ctx.actor.id.clone(),
                employee_id: ctx.actor.employee_id.clone(),
            };
            self.custody
                .check_save_in(tx, ctx.branch, &attempt, &actor, ctx.now)
                .await
        })
    }

    /// The draft named with the save, if any, is completed with the exchange's
    /// document number — but only the draft that fits the exchange (ticket
    /// #154). Not limited to live exchanges, because it is not today.
    fn after_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
        saved: &'a Transaction,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            let Some(attempt) = attempt_of(ctx) else {
                return Ok(());
            };
            if attempt.draft_id.is_none() {
                return Ok(());
            }
            self.custody
                .complete_draft_in(tx, ctx.branch, &attempt, &saved.document_ref)
                .await
        })
    }
}

/// The save as custody sees it: the figures of the form, the draft the staff
/// named with it, and what is already stored of this submission. `None` when
/// the form's currency is no currency — the save itself refuses it.
fn attempt_of(ctx: &SaveContext<'_>) -> Option<SaveAttempt> {
    let currency = CurrencyCode::parse(&ctx.form.currency).ok()?;
    let prior = match ctx.prior {
        None => PriorSave::None,
        Some(saved) if saved.recorded_by == ctx.actor.id => PriorSave::SavedByActor,
        Some(_) => PriorSave::SavedByOther,
    };
    Some(SaveAttempt {
        customer_id: ctx.form.customer_id.clone(),
        draft_id: ctx.extras.staff_draft_id.clone(),
        side: ctx.form.side,
        currency,
        amount: ctx.form.amount,
        prior,
    })
}
