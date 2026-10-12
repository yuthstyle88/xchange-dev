//! LY-3 #1385: `LoyaltyEffect` registered into the exchange-effect seam (design
//! `docs/design/blueprint/membership-points.md` §6.2, D-4, D-5, §9.1, §9.3).
//!
//! Enqueues `loyalty.exchange_completed` and `loyalty.exchange_reversed` outbox events
//! at the `Outbox` phase in the caller's transaction.

use std::sync::Arc;

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use xc_customer::CustomerService;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_kernel::money::Side;
use xc_loyalty::application::events::LoyaltyEvents;
use xc_loyalty::domain::facts::{CompletedFacts, CompletedLine, ReversedFacts};

use crate::application::workflows::exchange::effects::{
    BoxFuture, EffectPhase, ExchangeCompletedFacts, ExchangeEffect, PostedReversal,
};
use crate::bootstrap::AppState;

pub struct LoyaltyEffect;

impl ExchangeEffect for LoyaltyEffect {
    fn name(&self) -> &'static str {
        "loyalty"
    }

    fn phase(&self) -> EffectPhase {
        EffectPhase::Outbox
    }

    fn on_completed_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        facts: &'a ExchangeCompletedFacts,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            // D-5: Only LIVE exchanges of a customer with an ACTIVE verified PHONE earn points.
            if facts.mode != Mode::Live {
                return Ok(());
            }

            let Some(ref customer_id) = facts.customer_id else {
                return Ok(());
            };
            if customer_id.is_empty() {
                return Ok(());
            }

            // CU-L1 check in the caller's transaction (never decrypts)
            let has_verified_phone =
                CustomerService::has_verified_phone_in(tx, facts.organization_id, customer_id)
                    .await?;
            if !has_verified_phone {
                return Ok(());
            }

            // Fill facts gap by reading document_ref and lines from the transaction row in the same tx
            let document_ref: String =
                sqlx::query_scalar("SELECT document_ref FROM transactions WHERE id = $1")
                    .bind(facts.transaction_id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

            #[derive(sqlx::FromRow)]
            struct RawLine {
                currency: String,
                shop_side: String,
                quantity: Decimal,
                unit_size: Decimal,
                thb_amount: Decimal,
                settle_thb: Decimal,
                realized_margin_thb: Option<Decimal>,
            }

            let raw_lines: Vec<RawLine> = sqlx::query_as(
                "SELECT currency, shop_side, quantity, unit_size, thb_amount, settle_thb, realized_margin_thb \
                 FROM transaction_lines WHERE transaction_id = $1 ORDER BY line_no",
            )
            .bind(facts.transaction_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let lines = raw_lines
                .into_iter()
                .map(|line| {
                    let side = match line.shop_side.as_str() {
                        "BUY" => Side::Buy,
                        "SELL" => Side::Sell,
                        _ => Side::Buy,
                    };
                    CompletedLine {
                        currency: line.currency.trim().to_string(),
                        shop_side: side,
                        quantity: line.quantity,
                        unit_size: line.unit_size,
                        thb_amount: line.thb_amount,
                        settle_thb: line.settle_thb,
                        realized_margin_thb: line.realized_margin_thb,
                    }
                })
                .collect();

            let completed_facts = CompletedFacts {
                organization_id: facts.organization_id,
                transaction_id: facts.transaction_id,
                document_ref,
                customer_id: facts.customer_id.clone(),
                mode: facts.mode,
                branch_id: facts.branch_id.as_str().to_string(),
                completed_at: facts.completed_at,
                business_date: facts.business_date,
                reversible_until: facts.reversible_until,
                lines,
            };

            LoyaltyEvents::completed_in(tx, &completed_facts).await?;
            Ok(())
        })
    }

    fn on_reversed_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        reversal: &'a PostedReversal,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            // Ruling Q2 A: A reversal is enqueued only if its completion's loyalty outbox row exists.
            let completion_enqueued: bool = sqlx::query_scalar(
                "SELECT EXISTS( \
                     SELECT 1 FROM outbox_events \
                     WHERE organization_id = $1 \
                       AND event_type = 'loyalty.exchange_completed' \
                       AND aggregate_id = $2 \
                 )",
            )
            .bind(reversal.organization_id)
            .bind(reversal.transaction_id.to_string())
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            if !completion_enqueued {
                return Ok(());
            }

            // Read settled THB from transaction_lines
            let total_settled_thb: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(sum(settle_thb), 0) FROM transaction_lines WHERE transaction_id = $1",
            )
            .bind(reversal.transaction_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            // Read cumulative posted reversals on the original transaction
            let posted_thb: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(sum(amount_thb), 0) FROM reversals \
                 WHERE original_transaction_id = $1 AND (status = 'POSTED' OR id = $2)",
            )
            .bind(reversal.transaction_id)
            .bind(reversal.reversal_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let amount_thb = if posted_thb > Decimal::ZERO {
                posted_thb
            } else {
                reversal.amount_thb
            };
            let full = amount_thb >= total_settled_thb && total_settled_thb > Decimal::ZERO;

            let reversed_facts = ReversedFacts {
                organization_id: reversal.organization_id,
                reversal_id: reversal.reversal_id,
                transaction_id: reversal.transaction_id,
                customer_id: reversal.customer_id.clone(),
                posted_at: reversal.posted_at,
                amount_thb,
                full,
            };

            LoyaltyEvents::reversed_in(tx, &reversed_facts).await?;
            Ok(())
        })
    }
}

/// Registration hook picked up by `build.rs` (XS-0 #1375).
/// Ruling Q1 A: Always answers `Some`.
pub fn effect(_state: &AppState) -> Option<Arc<dyn ExchangeEffect>> {
    Some(Arc::new(LoyaltyEffect))
}
