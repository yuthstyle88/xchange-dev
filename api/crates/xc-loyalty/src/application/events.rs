//! `LoyaltyEvents` (design §6, D-4): writes the two loyalty outbox rows through
//! `OutboxWriter::enqueue_in`, in the caller's transaction.

use pos_error::AppResult;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use xc_kernel::Mode;
use xc_outbox::application::writer::OutboxWriter;

use crate::domain::events::{LOYALTY_COMPLETED, LOYALTY_REVERSED};
use crate::domain::facts::{CompletedFacts, ReversedFacts};

pub struct LoyaltyEvents;

impl LoyaltyEvents {
    /// LIVE with a customer ⇒ one `loyalty.exchange_completed` row, key `exchange:<transaction id>`,
    /// aggregate `transaction/<transaction id>`, payload `{"transactionId"}`. DEMO or no customer ⇒
    /// `Ok(())`, nothing written.
    pub async fn completed_in(
        tx: &mut Transaction<'_, Postgres>,
        facts: &CompletedFacts,
    ) -> AppResult<()> {
        if facts.mode != Mode::Live {
            return Ok(());
        }
        let Some(ref customer_id) = facts.customer_id else {
            return Ok(());
        };
        if customer_id.is_empty() {
            return Ok(());
        }
        let tx_id = facts.transaction_id.to_string();
        let org_id = facts.organization_id.to_string();
        let payload = json!({ "transactionId": tx_id });
        OutboxWriter::enqueue_in(
            tx,
            &org_id,
            &LOYALTY_COMPLETED,
            &tx_id,
            &format!("exchange:{tx_id}"),
            &payload,
        )
        .await?;
        Ok(())
    }

    /// One `loyalty.exchange_reversed` row, key `reversal:<reversal id>`, aggregate =
    /// `transaction/<original transaction id>`, payload `{"reversalId"}`.
    pub async fn reversed_in(
        tx: &mut Transaction<'_, Postgres>,
        facts: &ReversedFacts,
    ) -> AppResult<()> {
        let rev_id = facts.reversal_id.to_string();
        let original_tx_id = facts.transaction_id.to_string();
        let org_id = facts.organization_id.to_string();
        let payload = json!({ "reversalId": rev_id });
        OutboxWriter::enqueue_in(
            tx,
            &org_id,
            &LOYALTY_REVERSED,
            &original_tx_id,
            &format!("reversal:{rev_id}"),
            &payload,
        )
        .await?;
        Ok(())
    }
}
