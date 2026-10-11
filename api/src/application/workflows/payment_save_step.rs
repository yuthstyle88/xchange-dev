//! Payment settings part in the save of an exchange (PM-S, design §5.5.2).

use pos_error::AppError;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::entities::transaction::Transaction;

use crate::application::settings::PaymentSettingsService;
use crate::application::workflows::save_step::{SaveContext, SaveStep, StepFuture};

pub const UNAVAILABLE: &str = "Payment method unavailable. Please contact staff.";

/// Refuses a first LIVE save when the payment method is switched off in the branch.
#[derive(Clone)]
pub struct PaymentSaveStep {
    payments: PaymentSettingsService,
}

impl PaymentSaveStep {
    pub fn new(payments: PaymentSettingsService) -> Self {
        Self { payments }
    }
}

impl SaveStep for PaymentSaveStep {
    /// The check runs before anything is written (TX-5 #873, design §4.5): it refuses and writes nothing.
    fn before_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            if ctx.prior.is_none() && ctx.form.customer.mode == Mode::Live {
                let rules = self.payments.rules_in(tx, ctx.branch).await?;
                if !rules.allows(ctx.form.method.as_str(), ctx.form.side) {
                    return Err(AppError::Domain(UNAVAILABLE.to_string()));
                }
            }
            Ok(())
        })
    }

    fn after_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _ctx: &'a SaveContext<'a>,
        _saved: &'a Transaction,
    ) -> StepFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}
