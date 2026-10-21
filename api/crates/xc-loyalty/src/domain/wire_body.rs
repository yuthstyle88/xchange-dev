//! `LoyaltyWireBody` (design §4.5 `[required]`): the JSON delivered to Loyalty. Pure, no I/O.

use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{Value, json};
use uuid::Uuid;
use xc_kernel::money::Side;

use crate::domain::customer_ref::LoyaltyCustomerRef;
use crate::domain::facts::{CompletedFacts, ReversedFacts};

fn fixed(d: Decimal, dp: u32) -> String {
    format!(
        "{:.*}",
        dp as usize,
        d.round_dp_with_strategy(dp, RoundingStrategy::MidpointAwayFromZero)
    )
}

pub struct LoyaltyWireBody;

impl LoyaltyWireBody {
    /// `xchange.exchange_completed.v1`: `event_id` is the outbox row's id, `tenant_id` Loyalty's
    /// tenant (D-3), `phone` the customer's verified phone in E.164 (D-2).
    pub fn completed(
        event_id: Uuid,
        facts: &CompletedFacts,
        tenant_id: Uuid,
        phone: &str,
    ) -> Value {
        let customer_ref = LoyaltyCustomerRef::of(
            facts.organization_id,
            facts.customer_id.as_deref().unwrap_or(""),
        );

        let lines: Vec<Value> = facts
            .lines
            .iter()
            .map(|l| {
                let shop_side = match l.shop_side {
                    Side::Buy => "BUY",
                    Side::Sell => "SELL",
                };
                let realized_margin_thb = l.realized_margin_thb.map(|m| fixed(m, 2));
                json!({
                    "currency": l.currency,
                    "shopSide": shop_side,
                    "quantity": fixed(l.quantity, 4),
                    "unitSize": fixed(l.unit_size, 4),
                    "thbAmount": fixed(l.thb_amount, 2),
                    "settleThb": fixed(l.settle_thb, 2),
                    "realizedMarginThb": realized_margin_thb,
                })
            })
            .collect();

        json!({
            "id": event_id.to_string(),
            "type": "xchange.exchange_completed.v1",
            "data": {
                "branchId": facts.branch_id,
                "businessDate": facts.business_date.to_string(),
                "completedAt": facts.completed_at.to_rfc3339(),
                "customerRef": customer_ref.to_string(),
                "documentRef": facts.document_ref,
                "lines": lines,
                "phone": phone,
                "reversibleUntil": facts.reversible_until.to_rfc3339(),
                "tenantId": tenant_id.to_string(),
                "transactionId": facts.transaction_id.to_string(),
            }
        })
    }

    /// `xchange.exchange_reversed.v1`.
    pub fn reversed(event_id: Uuid, facts: &ReversedFacts, tenant_id: Uuid, phone: &str) -> Value {
        let customer_ref = LoyaltyCustomerRef::of(
            facts.organization_id,
            facts.customer_id.as_deref().unwrap_or(""),
        );

        json!({
            "id": event_id.to_string(),
            "type": "xchange.exchange_reversed.v1",
            "data": {
                "amountThb": fixed(facts.amount_thb, 2),
                "customerRef": customer_ref.to_string(),
                "full": facts.full,
                "phone": phone,
                "postedAt": facts.posted_at.to_rfc3339(),
                "reversalId": facts.reversal_id.to_string(),
                "tenantId": tenant_id.to_string(),
                "transactionId": facts.transaction_id.to_string(),
            }
        })
    }
}
