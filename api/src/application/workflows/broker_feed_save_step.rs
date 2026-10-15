//! FD-2 (#973, design `docs/design/blueprint/broker-seam.md` §6.2): the save step that writes the
//! Broker exchange feed row of a LIVE exchange, inside the save's transaction.

use rust_decimal::Decimal;
use uuid::Uuid;
use xc_exchange::domain::exchange::entities::transaction::Transaction as ExchangeTransaction;
use xc_kernel::mode::Mode;
use xc_referral::application::feed_writer::{CompletedExchange, ExchangeFeedWriter};
use xc_referral::domain::customer_keys::FeedCustomerKeys;
use xc_referral::domain::feed_value::FeedValue;
use xc_referral::infrastructure::booking_member_keys::BookingMemberKeys;

use crate::application::workflows::save_step::{SaveContext, SaveStep, StepFuture};

/// Writes one `broker_exchange_feed` row per LIVE exchange (design §6.2).
#[derive(Debug, Default)]
pub struct BrokerFeedSaveStep;

impl BrokerFeedSaveStep {
    pub fn new() -> Self {
        Self
    }
}

impl SaveStep for BrokerFeedSaveStep {
    fn before_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _ctx: &'a SaveContext<'a>,
    ) -> StepFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn after_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _ctx: &'a SaveContext<'a>,
        saved: &'a ExchangeTransaction,
    ) -> StepFuture<'a> {
        if saved.mode != Mode::Live {
            return Box::pin(async { Ok(()) });
        }

        Box::pin(async move {
            let row = sqlx::query_as::<_, (String, String)>(
                "SELECT t.id::text, b.organization_id::text \
                 FROM transactions t \
                 JOIN branches b ON b.id = t.branch_id \
                 WHERE t.document_ref = $1 AND t.branch_id = $2",
            )
            .bind(&saved.document_ref)
            .bind(saved.branch_id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let Some((tx_id_str, org_id_str)) = row else {
                return Err(pos_error::AppError::NotFound {
                    entity: "transactions",
                    id: saved.document_ref.clone(),
                });
            };

            let tx_id = Uuid::parse_str(&tx_id_str).map_err(|e| {
                pos_error::AppError::Internal(format!("invalid transaction uuid {tx_id_str}: {e}"))
            })?;

            let principal = Decimal::new(saved.principal.minor(), 2);
            let value_satang = FeedValue::of(principal)?;

            let keys = BookingMemberKeys.keys_in(tx, tx_id).await?;

            let exchange = CompletedExchange {
                transaction_id: tx_id,
                organization_id: org_id_str,
                branch_id: saved.branch_id.to_string(),
                keys,
                value_satang,
                completed_at: saved.recorded_at,
            };

            ExchangeFeedWriter::record_completed(tx, &exchange).await?;

            Ok(())
        })
    }
}
