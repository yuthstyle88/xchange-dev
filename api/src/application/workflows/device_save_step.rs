//! The devices' part in the save of an exchange (T8c-2b, `docs/design/xchange-v3.md` §5.4).

use rust_decimal::Decimal;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::entities::transaction::Transaction;

use crate::application::workflows::save_step::{SaveContext, SaveStep, StepFuture};
use crate::infrastructure::devices::device_commands::SavedExchange;
use crate::infrastructure::devices::device_service::DeviceService;

/// Refuses a cash exchange whose cash was not counted, and records the count, the link and the
/// receipt's print job on the save's own transaction.
#[derive(Clone)]
pub struct DeviceSaveStep {
    devices: DeviceService,
}

impl DeviceSaveStep {
    pub fn new(devices: DeviceService) -> Self {
        Self { devices }
    }
}

/// The saved exchange as the device log sees it. `Money::value` and `Rate::value`
/// are `pub(crate)`, so the amount returns from its hundredths and the rate from
/// its `Display` text — both exact, never a float.
fn saved_exchange(saved: &Transaction) -> SavedExchange {
    SavedExchange {
        cash: saved.method.is_cash(),
        submission_id: saved.submission_id.clone(),
        document_ref: saved.document_ref.clone(),
        currency: saved.currency.clone(),
        amount: Decimal::new(saved.amount.minor(), 2),
        rate: Decimal::from_str_exact(&saved.rate.to_string())
            .expect("a rate's Display text is an exact decimal"),
        passport_key: Some(saved.passport_key.clone()),
        slip_key: saved.slip_key.clone(),
    }
}

impl SaveStep for DeviceSaveStep {
    /// A `DEMO` exchange and a repeat of the same submission are not checked
    /// (§5.4): the repeat answers the first save's document, so asking for the
    /// count again could only refuse a save that already happened. A live first
    /// save of cash must have its cash counted.
    fn before_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            if ctx.form.customer.mode != Mode::Live || ctx.prior.is_some() {
                return Ok(());
            }
            DeviceService::require_count_confirmed(
                ctx.form.method.is_cash(),
                ctx.extras.count_confirmed,
            )
        })
    }

    /// The count line when cash, the link line and one `PRINT_RECEIPT` job, on
    /// the save's own transaction and with the actor's id. A `DEMO` exchange
    /// writes nothing; a repeat adds nothing the second time (`after_save_in`'s
    /// keys).
    fn after_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
        saved: &'a Transaction,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            if ctx.form.customer.mode != Mode::Live {
                return Ok(());
            }
            self.devices
                .after_save_in(
                    tx,
                    ctx.branch,
                    &saved_exchange(saved),
                    &ctx.actor.id,
                    ctx.now,
                )
                .await
        })
    }
}
