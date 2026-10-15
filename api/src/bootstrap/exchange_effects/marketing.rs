//! Marketing exchange effect (MK-5, design `marketing-campaigns.md` §5, §6, §7).

use std::sync::Arc;

use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::mode::Mode;
use xc_marketing::application::redeemer::BenefitRedeemer;
use xc_marketing::infrastructure::pg_budget_ledger::{BudgetLedger, CompletedExchange};

use crate::application::workflows::exchange::effects::{
    BoxFuture, EffectPhase, ExchangeCompletedFacts, ExchangeEffect, PostedReversal,
};
use crate::bootstrap::AppState;

pub struct MarketingEffect {
    redeemer: Arc<BenefitRedeemer>,
}

impl MarketingEffect {
    pub fn new(redeemer: Arc<BenefitRedeemer>) -> Self {
        Self { redeemer }
    }
}

impl ExchangeEffect for MarketingEffect {
    fn name(&self) -> &'static str {
        "marketing"
    }

    fn phase(&self) -> EffectPhase {
        EffectPhase::Redeem
    }

    fn on_completed_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        e: &'a ExchangeCompletedFacts,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if e.mode == Mode::Demo {
                return Ok(());
            }

            let referral_attribution_id: Option<Uuid> = match e.customer_id.as_deref() {
                Some(cid) => sqlx::query_scalar(
                    "SELECT id FROM referral_attributions \
                     WHERE customer_id = $1 AND status = 'ACTIVE' AND captured_at <= $2 \
                     ORDER BY captured_at DESC LIMIT 1",
                )
                .bind(cid)
                .bind(e.completed_at)
                .fetch_optional(&mut **tx)
                .await
                .map_err(pos_error::AppError::Database)?,
                None => None,
            };

            let exchange = CompletedExchange {
                transaction_id: e.transaction_id,
                customer_id: e.customer_id.clone(),
                referral_attribution_id,
            };

            match self.redeemer.redeem_in(tx, &exchange, e.quote_id).await {
                Ok(_) => Ok(()),
                Err(err) => match err {
                    xc_marketing::domain::coupon::benefit_error::BenefitError::Store(app_err) => {
                        Err(app_err)
                    }
                    other => Err(pos_error::AppError::Domain(other.to_string())),
                },
            }
        })
    }

    fn on_reversed_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        r: &'a PostedReversal,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if r.mode == Mode::Demo {
                return Ok(());
            }

            let reversal = xc_marketing::application::redeemer::PostedReversal {
                reversal_id: r.reversal_id,
                transaction_id: r.transaction_id,
            };

            match self.redeemer.adjust_in(tx, &reversal).await {
                Ok(_) => Ok(()),
                Err(err) => match err {
                    xc_marketing::domain::coupon::benefit_error::BenefitError::Store(app_err) => {
                        Err(app_err)
                    }
                    other => Err(pos_error::AppError::Domain(other.to_string())),
                },
            }
        })
    }
}

pub fn effect(state: &AppState) -> Option<Arc<dyn ExchangeEffect>> {
    let ledger = Arc::new(BudgetLedger::new(state.db.clone()));
    let redeemer = Arc::new(BenefitRedeemer::new(ledger));
    Some(Arc::new(MarketingEffect::new(redeemer)))
}
