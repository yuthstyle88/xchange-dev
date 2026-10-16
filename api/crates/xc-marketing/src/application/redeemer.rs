//! `BenefitRedeemer` (design D-8, D-16, §4.2, owner M3): the completion's *redeem benefit* step and the reversal's
//! *benefit adjustment*, reached through the exchange-effect seam (MK-5).

use std::sync::Arc;

use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::coupon::benefit_error::BenefitError;
use crate::infrastructure::pg_budget_ledger::{BudgetLedger, CompletedExchange};

/// A posted reversal of an exchange (#552).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostedReversal {
    pub reversal_id: Uuid,
    /// The reversed exchange.
    pub transaction_id: Uuid,
}

pub struct BenefitRedeemer {
    ledger: Arc<BudgetLedger>,
}

type QuoteReservationRow = (
    Uuid,
    Uuid,
    Uuid,
    Option<Uuid>,
    Option<String>,
    Decimal,
    String,
    bool,
);

impl BenefitRedeemer {
    pub fn new(ledger: Arc<BudgetLedger>) -> Self {
        Self { ledger }
    }

    /// D-8: consumes every reservation of `quote_id` for `exchange`. A reservation that expired (still `RESERVED`, or
    /// already set `RELEASED` by a lazy release) is re-reserved under the lock, or the step refuses
    /// `BenefitLimitReached` (the caller rolls the completion back). Returns the new redemptions' ids.
    pub async fn redeem_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        exchange: &CompletedExchange,
        quote_id: Uuid,
    ) -> Result<Vec<Uuid>, BenefitError> {
        let reservations: Vec<QuoteReservationRow> = sqlx::query_as(
            "SELECT id, rule_id, campaign_version_id, coupon_id, customer_id, amount_thb, status, \
             (expires_at <= now()) AS is_expired \
             FROM benefit_reservations WHERE quote_id = $1 ORDER BY created_at ASC",
        )
        .bind(quote_id)
        .fetch_all(&mut **tx)
        .await?;

        if reservations.is_empty() {
            return Ok(Vec::new());
        }

        let mut redemption_ids = Vec::new();

        for (
            res_id,
            rule_id,
            version_id,
            coupon_id,
            cust_id,
            amount_thb,
            mut status,
            mut is_expired,
        ) in reservations
        {
            // Check if already redeemed for this exchange/transaction
            let already_redeemed: Option<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM benefit_redemptions \
                 WHERE reservation_id = $1 OR (rule_id = $2 AND transaction_id = $3)",
            )
            .bind(res_id)
            .bind(rule_id)
            .bind(exchange.transaction_id)
            .fetch_optional(&mut **tx)
            .await?;

            if already_redeemed.is_some() {
                continue;
            }

            // Lock reservation row
            let locked: Option<(String, bool)> = sqlx::query_as(
                "SELECT status, (expires_at <= now()) AS is_expired \
                 FROM benefit_reservations WHERE id = $1 FOR UPDATE",
            )
            .bind(res_id)
            .fetch_optional(&mut **tx)
            .await?;

            if let Some((s, exp)) = locked {
                status = s;
                is_expired = exp;
            }

            if status == "CONSUMED" {
                continue;
            }

            if status == "RELEASED" || is_expired {
                // Re-reserve under lock
                // 1. Lock campaign_budgets
                let (consumed_thb, used_count): (Decimal, i32) = sqlx::query_as(
                    "SELECT consumed_thb, used_count FROM campaign_budgets WHERE campaign_version_id = $1 FOR UPDATE",
                )
                .bind(version_id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(BenefitError::NotFound)?;

                // 2. Lock coupon if present
                if let Some(cid) = coupon_id {
                    let coupon_row: Option<(String, Option<String>, bool)> = sqlx::query_as(
                        "SELECT status, customer_id, (expires_at <= now()) AS is_expired \
                         FROM coupons WHERE id = $1 FOR UPDATE",
                    )
                    .bind(cid)
                    .fetch_optional(&mut **tx)
                    .await?;

                    match coupon_row {
                        Some((c_status, c_cust, c_expired)) => {
                            if c_status != "ISSUED" || c_expired {
                                return Err(BenefitError::BenefitLimitReached);
                            }
                            if c_cust.is_some()
                                && c_cust.as_deref() != exchange.customer_id.as_deref()
                            {
                                return Err(BenefitError::BenefitLimitReached);
                            }
                        }
                        None => return Err(BenefitError::BenefitLimitReached),
                    }

                    // Lazy release expired reservations for this coupon
                    sqlx::query(
                        "UPDATE benefit_reservations \
                         SET status = 'RELEASED' \
                         WHERE coupon_id = $1 AND status = 'RESERVED' AND expires_at <= now()",
                    )
                    .bind(cid)
                    .execute(&mut **tx)
                    .await?;

                    let (live_coupon_count,): (i64,) = sqlx::query_as(
                        "SELECT count(*) FROM benefit_reservations \
                         WHERE coupon_id = $1 AND status = 'RESERVED' AND expires_at > now() AND id <> $2",
                    )
                    .bind(cid)
                    .bind(res_id)
                    .fetch_one(&mut **tx)
                    .await?;

                    if live_coupon_count > 0 {
                        return Err(BenefitError::BenefitLimitReached);
                    }
                }

                // 3. Lazy release expired reservations for this campaign version
                sqlx::query(
                    "UPDATE benefit_reservations \
                     SET status = 'RELEASED' \
                     WHERE campaign_version_id = $1 AND status = 'RESERVED' AND expires_at <= now()",
                )
                .bind(version_id)
                .execute(&mut **tx)
                .await?;

                // 4. Check max_uses, the per-customer cap and available budget
                let (budget_thb, max_uses): (Decimal, Option<i32>) = sqlx::query_as(
                    "SELECT budget_thb, max_uses FROM campaign_versions WHERE id = $1",
                )
                .bind(version_id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(BenefitError::NotFound)?;

                let (live_count, live_reservations_sum): (i64, Decimal) = sqlx::query_as(
                    "SELECT count(*), COALESCE(SUM(amount_thb), 0) FROM benefit_reservations \
                     WHERE campaign_version_id = $1 AND status = 'RESERVED' AND expires_at > now() AND id != $2",
                )
                .bind(version_id)
                .bind(res_id)
                .fetch_one(&mut **tx)
                .await?;

                if let Some(max) = max_uses {
                    if (used_count as i64 + live_count) >= max as i64 {
                        return Err(BenefitError::BenefitLimitReached);
                    }
                }

                if self
                    .ledger
                    .per_customer_cap_reached_in(tx, rule_id, cust_id.as_deref())
                    .await?
                {
                    return Err(BenefitError::BenefitLimitReached);
                }

                let available = budget_thb - consumed_thb - live_reservations_sum;
                if available < amount_thb {
                    return Err(BenefitError::BenefitLimitReached);
                }

                // 5. Re-reserve
                sqlx::query(
                    "UPDATE benefit_reservations \
                     SET status = 'RESERVED', expires_at = now() + interval '15 minutes' \
                     WHERE id = $1",
                )
                .bind(res_id)
                .execute(&mut **tx)
                .await?;
            }

            let redemption_id = self.ledger.consume_in(tx, res_id, exchange).await?;
            redemption_ids.push(redemption_id);
        }

        Ok(redemption_ids)
    }

    /// Owner M3: the exchange's redemptions become `REVERSED` (with `reversal_id`); budget and use count are returned;
    /// a single-use coupon goes `REDEEMED → ISSUED` only if its version is still ACTIVE and the coupon unexpired.
    /// Returns how many redemptions were reversed.
    pub async fn adjust_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        reversal: &PostedReversal,
    ) -> Result<u32, BenefitError> {
        let redemptions: Vec<(Uuid, Uuid, Option<Uuid>, Decimal)> = sqlx::query_as(
            "SELECT id, campaign_version_id, coupon_id, amount_thb \
             FROM benefit_redemptions WHERE transaction_id = $1 AND status = 'REDEEMED' FOR UPDATE",
        )
        .bind(reversal.transaction_id)
        .fetch_all(&mut **tx)
        .await?;

        if redemptions.is_empty() {
            return Ok(0);
        }

        let count = redemptions.len() as u32;

        for (redemption_id, version_id, coupon_id, amount_thb) in redemptions {
            // Update redemption status to REVERSED with reversal_id
            sqlx::query(
                "UPDATE benefit_redemptions \
                 SET status = 'REVERSED', reversal_id = $1 \
                 WHERE id = $2",
            )
            .bind(reversal.reversal_id)
            .bind(redemption_id)
            .execute(&mut **tx)
            .await?;

            // Return budget and use count
            sqlx::query(
                "UPDATE campaign_budgets \
                 SET consumed_thb = GREATEST(0, consumed_thb - $1), \
                     used_count = GREATEST(0, used_count - 1) \
                 WHERE campaign_version_id = $2",
            )
            .bind(amount_thb)
            .bind(version_id)
            .execute(&mut **tx)
            .await?;

            // If coupon attached, check if version is still ACTIVE and coupon unexpired
            if let Some(cid) = coupon_id {
                let is_version_active: (bool,) = sqlx::query_as(
                    "SELECT (starts_at <= now() AND now() < ends_at AND status = 'APPROVED') \
                     FROM campaign_versions WHERE id = $1",
                )
                .bind(version_id)
                .fetch_one(&mut **tx)
                .await?;

                let is_coupon_unexpired: (bool,) = sqlx::query_as(
                    "SELECT (expires_at > now()) FROM coupons WHERE id = $1 FOR UPDATE",
                )
                .bind(cid)
                .fetch_one(&mut **tx)
                .await?;

                if is_version_active.0 && is_coupon_unexpired.0 {
                    sqlx::query("UPDATE coupons SET status = 'ISSUED' WHERE id = $1")
                        .bind(cid)
                        .execute(&mut **tx)
                        .await?;
                }
            }
        }

        Ok(count)
    }
}
