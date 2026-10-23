//! The kiosk's part in the save of an exchange.

use xc_auth::domain::access::{Role as AuthRole, StandingKind};
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_kernel::mode::Mode;
use xc_kernel::money::CurrencyCode;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use xc_kiosk::domain::kiosk::value_objects::kiosk_role::KioskRole;
use xc_kiosk::domain::kiosk::value_objects::save_attempt::SaveAttempt;
use xc_kiosk::domain::kiosk::value_objects::transaction_link::TransactionLink;

use crate::application::workflows::save_step::{SaveActor, SaveContext, SaveStep, StepFuture};

/// Checks a save that names a kiosk request against that request, and
/// completes the request with the exchange it became.
#[derive(Clone)]
pub struct KioskSaveStep {
    kiosk: KioskService,
}

impl KioskSaveStep {
    pub fn new(kiosk: KioskService) -> Self {
        Self { kiosk }
    }
}

/// The kiosk request the save names; the empty text names none.
fn request_id<'a>(ctx: &SaveContext<'a>) -> Option<&'a str> {
    ctx.extras
        .kiosk_request_id
        .as_deref()
        .filter(|id| !id.is_empty())
}

/// The kiosk role of a standing. A role with no part at the kiosk desk is `Other`.
fn kiosk_role(kind: StandingKind) -> KioskRole {
    match kind {
        StandingKind::Owner => KioskRole::Owner,
        StandingKind::Staff(AuthRole::Teller) => KioskRole::Front,
        StandingKind::Staff(AuthRole::BranchManager) => KioskRole::Manager,
        StandingKind::Staff(AuthRole::RateManager) => KioskRole::Rates,
        _ => KioskRole::Other,
    }
}

/// The saver as the kiosk knows them. The kiosk's `username` holds the
/// actor's id: a request is held by the id of the employee who accepted it.
fn kiosk_actor(actor: &SaveActor) -> KioskActor {
    KioskActor::Staff {
        username: actor.id.clone(),
        role: kiosk_role(actor.standing()),
    }
}

impl SaveStep for KioskSaveStep {
    /// Only when the save names a request: the request must be ready and the
    /// saver's, the save live, and side, currency, amount and method those the
    /// customer confirmed. The kiosk's refusal is passed on unchanged.
    fn before_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            let Some(id) = request_id(ctx) else {
                return Ok(());
            };
            // A text that is no currency cannot be the request's. The kiosk
            // never offers the baht (`KioskCurrencies`, and configuring a
            // kiosk currency refuses it), so the baht stands in for it and is
            // refused as a difference, after the checks that come before it.
            let currency = CurrencyCode::parse(&ctx.form.currency).unwrap_or(CurrencyCode::THB);
            let attempt = SaveAttempt {
                live: ctx.form.customer.mode == Mode::Live,
                side: ctx.form.side,
                currency,
                amount: ctx.form.amount,
                method: ctx.form.method.as_str().to_owned(),
                submission_id: ctx.form.submission_id.clone(),
            };
            self.kiosk
                .check_save_in(tx, ctx.branch, id, &attempt, &kiosk_actor(ctx.actor))
                .await
        })
    }

    /// The named request is linked to the exchange and completed; on a repeat
    /// it is already completed by this submission and nothing changes.
    fn after_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
        saved: &'a Transaction,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            let Some(id) = request_id(ctx) else {
                return Ok(());
            };
            let link = TransactionLink {
                submission_id: saved.submission_id.clone(),
                document_ref: saved.document_ref.clone(),
            };
            self.kiosk
                .complete_in(tx, ctx.branch, id, &link, &kiosk_actor(ctx.actor), ctx.now)
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kiosk role of every standing: unchanged from the v3 role it maps from (OR-7 R7 #3167).
    #[test]
    fn kiosk_role_of_each_standing() {
        for (kind, role) in [
            (StandingKind::Owner, KioskRole::Owner),
            (StandingKind::Staff(AuthRole::Teller), KioskRole::Front),
            (
                StandingKind::Staff(AuthRole::BranchManager),
                KioskRole::Manager,
            ),
            (StandingKind::Staff(AuthRole::RateManager), KioskRole::Rates),
            (StandingKind::Staff(AuthRole::Finance), KioskRole::Other),
            (StandingKind::Staff(AuthRole::Admin), KioskRole::Other),
            (
                StandingKind::Staff(AuthRole::VerificationReviewer),
                KioskRole::Other,
            ),
        ] {
            assert_eq!(kiosk_role(kind), role, "{kind:?}");
        }
    }
}
