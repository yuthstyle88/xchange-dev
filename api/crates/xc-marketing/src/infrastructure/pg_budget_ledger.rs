//! `BudgetLedger` (design D-8, §4.1 `campaign_budgets` / `benefit_reservations` / `benefit_redemptions`): reserve at
//! quote under the budget row lock, consume at completion, release expired reservations lazily.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::benefit::rule::BenefitCaps;
use crate::domain::coupon::benefit_error::BenefitError;

/// One benefit a quote wants to hold (one `benefit_reservations` row).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReserveRequest {
    pub organization_id: Uuid,
    pub quote_id: Uuid,
    pub rule_id: Uuid,
    pub campaign_version_id: Uuid,
    pub coupon_id: Option<Uuid>,
    pub customer_id: Option<String>,
    pub amount_thb: Decimal,
    /// The quote's `expires_at` (D-8).
    pub expires_at: DateTime<Utc>,
}

/// Why a benefit is not offered (§4.3, §4.5): a skip for the caller, not an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    BudgetExhausted,
    CapReached,
    CouponInvalid,
}

impl SkipReason {
    /// `budgetExhausted`, `capReached`, `couponInvalid`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BudgetExhausted => "budgetExhausted",
            Self::CapReached => "capReached",
            Self::CouponInvalid => "couponInvalid",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReserveOutcome {
    /// The new `RESERVED` row's id.
    Reserved(Uuid),
    Skipped(SkipReason),
}

/// The exchange a reservation is consumed for (one `benefit_redemptions` row).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedExchange {
    pub transaction_id: Uuid,
    pub customer_id: Option<String>,
    pub referral_attribution_id: Option<Uuid>,
}

pub struct BudgetLedger {
    pool: PgPool,
}

type ReservationRow = (
    Uuid,
    Uuid,
    Uuid,
    Option<Uuid>,
    Option<String>,
    Decimal,
    String,
);

impl BudgetLedger {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// D-8: under `campaign_budgets … FOR UPDATE` (and the coupon's row lock when `coupon_id` is set) inserts a
    /// `RESERVED` row only if `budget_thb − consumed_thb − Σ live RESERVED ≥ amount_thb` and the coupon is usable;
    /// expired `RESERVED` rows of the version (and the coupon's) are set `RELEASED` first.
    pub async fn reserve_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        request: &ReserveRequest,
    ) -> Result<ReserveOutcome, BenefitError> {
        // 1. Lazy release expired reservations for this campaign version
        sqlx::query(
            "UPDATE benefit_reservations \
             SET status = 'RELEASED' \
             WHERE campaign_version_id = $1 AND status = 'RESERVED' AND expires_at <= now()",
        )
        .bind(request.campaign_version_id)
        .execute(&mut **tx)
        .await?;

        // 2. Lock the campaign budget row
        let (consumed_thb, used_count): (Decimal, i32) = sqlx::query_as(
            "SELECT consumed_thb, used_count FROM campaign_budgets WHERE campaign_version_id = $1 FOR UPDATE",
        )
        .bind(request.campaign_version_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(BenefitError::NotFound)?;

        // 3. If coupon is specified, lock coupon row and validate
        if let Some(coupon_id) = request.coupon_id {
            // Lazy release expired reservations for this coupon first
            sqlx::query(
                "UPDATE benefit_reservations \
                 SET status = 'RELEASED' \
                 WHERE coupon_id = $1 AND status = 'RESERVED' AND expires_at <= now()",
            )
            .bind(coupon_id)
            .execute(&mut **tx)
            .await?;

            let coupon_row: Option<(String, Option<String>, DateTime<Utc>, bool)> = sqlx::query_as(
                "SELECT status, customer_id, expires_at, (expires_at <= now()) AS is_expired \
                 FROM coupons WHERE id = $1 FOR UPDATE",
            )
            .bind(coupon_id)
            .fetch_optional(&mut **tx)
            .await?;

            let (status, customer_id, _expires_at, is_expired) = match coupon_row {
                Some(row) => row,
                None => return Ok(ReserveOutcome::Skipped(SkipReason::CouponInvalid)),
            };

            if status != "ISSUED" || is_expired {
                return Ok(ReserveOutcome::Skipped(SkipReason::CouponInvalid));
            }

            // If personal coupon, must match quote's customer
            if customer_id.is_some() && customer_id.as_deref() != request.customer_id.as_deref() {
                return Ok(ReserveOutcome::Skipped(SkipReason::CouponInvalid));
            }

            // Check if there is already an active RESERVED row for this coupon
            let live_count: (i64,) = sqlx::query_as(
                "SELECT count(*) FROM benefit_reservations \
                 WHERE coupon_id = $1 AND status = 'RESERVED' AND expires_at > now()",
            )
            .bind(coupon_id)
            .fetch_one(&mut **tx)
            .await?;

            if live_count.0 > 0 {
                return Ok(ReserveOutcome::Skipped(SkipReason::CouponInvalid));
            }
        }

        // 4. Fetch version budget and max_uses
        let (budget_thb, max_uses): (Decimal, Option<i32>) =
            sqlx::query_as("SELECT budget_thb, max_uses FROM campaign_versions WHERE id = $1")
                .bind(request.campaign_version_id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(BenefitError::NotFound)?;

        // 5. Calculate live reservations count and sum
        let (live_count, live_reservations_sum): (i64, Decimal) = sqlx::query_as(
            "SELECT count(*), COALESCE(SUM(amount_thb), 0) FROM benefit_reservations \
             WHERE campaign_version_id = $1 AND status = 'RESERVED' AND expires_at > now()",
        )
        .bind(request.campaign_version_id)
        .fetch_one(&mut **tx)
        .await?;

        // 6. Check max_uses cap
        if let Some(max) = max_uses {
            if (used_count as i64 + live_count) >= max as i64 {
                return Ok(ReserveOutcome::Skipped(SkipReason::CapReached));
            }
        }

        // 7. Check per-customer caps
        if self
            .per_customer_cap_reached_in(tx, request.rule_id, request.customer_id.as_deref())
            .await?
        {
            return Ok(ReserveOutcome::Skipped(SkipReason::CapReached));
        }

        // 8. Check budget availability
        let available = budget_thb - consumed_thb - live_reservations_sum;
        if available < request.amount_thb {
            return Ok(ReserveOutcome::Skipped(SkipReason::BudgetExhausted));
        }

        // 6. Insert reservation
        let reservation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO benefit_reservations ( \
                 id, organization_id, rule_id, campaign_version_id, quote_id, \
                 coupon_id, customer_id, amount_thb, status, expires_at, created_at \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'RESERVED', $9, now())",
        )
        .bind(reservation_id)
        .bind(request.organization_id)
        .bind(request.rule_id)
        .bind(request.campaign_version_id)
        .bind(request.quote_id)
        .bind(request.coupon_id)
        .bind(&request.customer_id)
        .bind(request.amount_thb)
        .bind(request.expires_at)
        .execute(&mut **tx)
        .await?;

        Ok(ReserveOutcome::Reserved(reservation_id))
    }

    /// The rule's `per_customer_uses` cap is reached for `customer_id`: its `REDEEMED` redemptions plus live
    /// `RESERVED` reservations (within `per_customer_period_days` when set) leave no use. No cap, or no customer, is
    /// never reached. Shared by `reserve_in` and the completion's re-reserve (`BenefitRedeemer::redeem_in`).
    pub async fn per_customer_cap_reached_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        rule_id: Uuid,
        customer_id: Option<&str>,
    ) -> Result<bool, BenefitError> {
        let (caps_json,): (serde_json::Value,) =
            sqlx::query_as("SELECT caps FROM benefit_rules WHERE id = $1")
                .bind(rule_id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(BenefitError::NotFound)?;
        let caps: BenefitCaps = serde_json::from_value(caps_json)
            .map_err(|e| pos_error::AppError::Internal(format!("failed to decode caps: {e}")))?;

        let Some(per_customer_uses) = caps.per_customer_uses else {
            return Ok(false);
        };
        let Some(customer_id) = customer_id.filter(|s| !s.is_empty()) else {
            return Ok(false);
        };
        let since: Option<DateTime<Utc>> = caps
            .per_customer_period_days
            .map(|days| Utc::now() - chrono::Duration::days(days as i64));

        let (cust_uses,): (i64,) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM benefit_redemptions \
                  WHERE rule_id = $1 AND customer_id = $2 AND status = 'REDEEMED' \
                    AND ($3::timestamptz IS NULL OR redeemed_at >= $3)) \
                 + \
                 (SELECT count(*) FROM benefit_reservations \
                  WHERE rule_id = $1 AND customer_id = $2 AND status = 'RESERVED' AND expires_at > now() \
                    AND ($3::timestamptz IS NULL OR created_at >= $3))",
        )
        .bind(rule_id)
        .bind(customer_id)
        .bind(since)
        .fetch_one(&mut **tx)
        .await?;

        Ok(cust_uses >= per_customer_uses as i64)
    }

    /// D-8: `RESERVED` → `CONSUMED`, one `benefit_redemptions` row, `consumed_thb` / `used_count` up, a coupon
    /// `REDEEMED`. Returns the redemption's id. A reservation that is not `RESERVED` is refused.
    pub async fn consume_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        reservation_id: Uuid,
        exchange: &CompletedExchange,
    ) -> Result<Uuid, BenefitError> {
        let reservation: Option<ReservationRow> = sqlx::query_as(
            "SELECT organization_id, rule_id, campaign_version_id, coupon_id, customer_id, amount_thb, status \
             FROM benefit_reservations WHERE id = $1 FOR UPDATE",
        )
        .bind(reservation_id)
        .fetch_optional(&mut **tx)
        .await?;

        let (org_id, rule_id, version_id, coupon_id, _cust_id, amount_thb, status) =
            match reservation {
                Some(res) => res,
                None => return Err(BenefitError::NotFound),
            };

        if status != "RESERVED" {
            return Err(BenefitError::StaleReservation);
        }

        let (benefit_class,): (String,) =
            sqlx::query_as("SELECT benefit_class FROM benefit_rules WHERE id = $1")
                .bind(rule_id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(BenefitError::NotFound)?;

        // Update reservation to CONSUMED
        sqlx::query("UPDATE benefit_reservations SET status = 'CONSUMED' WHERE id = $1")
            .bind(reservation_id)
            .execute(&mut **tx)
            .await?;

        // If coupon attached, update coupon to REDEEMED
        if let Some(cid) = coupon_id {
            sqlx::query("UPDATE coupons SET status = 'REDEEMED' WHERE id = $1")
                .bind(cid)
                .execute(&mut **tx)
                .await?;
        }

        // Update campaign_budgets
        sqlx::query(
            "UPDATE campaign_budgets \
             SET consumed_thb = consumed_thb + $1, used_count = used_count + 1 \
             WHERE campaign_version_id = $2",
        )
        .bind(amount_thb)
        .bind(version_id)
        .execute(&mut **tx)
        .await?;

        // Insert benefit_redemptions
        let redemption_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO benefit_redemptions ( \
                 id, organization_id, rule_id, campaign_version_id, reservation_id, coupon_id, \
                 transaction_id, customer_id, referral_attribution_id, benefit_class, amount_thb, \
                 status, reversal_id, redeemed_at \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'REDEEMED', NULL, now())",
        )
        .bind(redemption_id)
        .bind(org_id)
        .bind(rule_id)
        .bind(version_id)
        .bind(reservation_id)
        .bind(coupon_id)
        .bind(exchange.transaction_id)
        .bind(&exchange.customer_id)
        .bind(exchange.referral_attribution_id)
        .bind(benefit_class)
        .bind(amount_thb)
        .execute(&mut **tx)
        .await?;

        Ok(redemption_id)
    }

    /// D-8: sets the version's `RESERVED` rows with `expires_at ≤ now()` to `RELEASED`; returns how many.
    pub async fn release_expired_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        campaign_version_id: Uuid,
    ) -> Result<u32, BenefitError> {
        let result = sqlx::query(
            "UPDATE benefit_reservations \
             SET status = 'RELEASED' \
             WHERE campaign_version_id = $1 AND status = 'RESERVED' AND expires_at <= now()",
        )
        .bind(campaign_version_id)
        .execute(&mut **tx)
        .await?;

        Ok(result.rows_affected() as u32)
    }
}
