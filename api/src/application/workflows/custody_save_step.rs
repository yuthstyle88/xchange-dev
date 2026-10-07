//! Custody's part in the save of an exchange.

use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_kernel::money::CurrencyCode;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::domain::custody::value_objects::custody_actor::CustodyActor;
use xc_shift::domain::custody::value_objects::save_attempt::{PriorSave, SaveAttempt};

use crate::application::custody_role::custody_role_of;
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
    /// of `CustodyMachine::check_save`. A currency that is no currency is left
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
            let Ok(currency) = CurrencyCode::parse(&ctx.form.currency) else {
                return Ok(());
            };
            let prior = match ctx.prior {
                None => PriorSave::None,
                Some(saved) if saved.recorded_by == ctx.actor.id => PriorSave::SavedByActor,
                Some(_) => PriorSave::SavedByOther,
            };
            let attempt = SaveAttempt {
                customer_id: ctx.form.customer_id.clone(),
                draft_id: ctx.extras.staff_draft_id.clone(),
                side: ctx.form.side,
                currency,
                amount: ctx.form.amount,
                prior,
            };
            let actor = CustodyActor {
                id: ctx.actor.id.clone(),
                employee_id: ctx.actor.employee_id.clone(),
                role: custody_role_of(ctx.actor.role),
            };
            self.custody
                .check_save_in(tx, ctx.branch, &attempt, &actor, ctx.now)
                .await
        })
    }

    /// The draft named with the save, if any, is completed with the exchange's
    /// document number.
    fn after_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
        saved: &'a Transaction,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            match &ctx.extras.staff_draft_id {
                Some(draft_id) => {
                    self.custody
                        .complete_draft_in(tx, ctx.branch, draft_id, &saved.document_ref)
                        .await
                }
                None => Ok(()),
            }
        })
    }
}
