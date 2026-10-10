//! `PlannerPreview` (design P5, §4.3): a fresh estimate of a plan from the branch's current
//! publication. It writes nothing: no quote, no stock, no booking.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use xc_kernel::money::{ExchangeArithmetic, Money, Rate, UnitSize};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::publication::published_rates::{LineSelector, PublishedRates};
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

use crate::domain::app_error::CustomerAppError;
use crate::domain::planner::planner_draft::{DraftAmount, DraftPlan};

/// An estimate. Never a commitment: the rate is fixed at the desk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerEstimate {
    /// `ExchangeArithmetic::thb(quantity, rate, unit_size)`, as the desk quote computes it.
    pub thb: Money,
    /// The published rate of the shop's side (`CustomerSide::to_shop`).
    pub rate: Rate,
    pub unit_size: UnitSize,
    /// The published line is stale and its stale policy admitted the amount.
    pub stale: bool,
    pub source_date: Option<NaiveDate>,
}

pub struct PlannerPreview {
    published_rates: PgPublishedRates,
}

impl PlannerPreview {
    /// Wires `PgPublishedRates` itself.
    pub fn new(pool: PgPool) -> Self {
        Self {
            published_rates: PgPublishedRates::new(pool),
        }
    }

    /// The estimate of `plan` at its branch, read in `tx`, which it never writes to.
    ///
    /// - `plan.branch_id` is `None` → `validationFailed` on `branch`.
    /// - The line is the one `line_for` picks for the currency, the shop's side and the quantity
    ///   in the branch's default denomination group, `ALL` until groups exist (rate-feed-connector
    ///   §9.3, as the kiosk; #994 ruling option A), so a tiered line prices at the quantity's tier
    ///   exactly as the desk quote does.
    /// - No such line, or a stale line its stale policy refuses (`StalePolicy::admits`) →
    ///   `rateUnavailable`, as `QuoteService::quote_in`.
    /// - [proposed] a THB-budget plan is not pinned by the up-front tests.
    pub async fn estimate(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        plan: &DraftPlan,
        now: DateTime<Utc>,
    ) -> Result<PlannerEstimate, CustomerAppError> {
        let _ = now; // staleness is read at line time by `PublishedRates`, as `QuoteService` does
        let branch = plan
            .branch_id
            .as_ref()
            .ok_or(CustomerAppError::ValidationFailed { field: "branch" })?;
        let quantity = match plan.amount {
            DraftAmount::Quantity(q) => q,
            // [proposed] no rule turns a THB budget into a foreign amount yet (#994 tests comment)
            DraftAmount::BudgetThb(_) => {
                return Err(CustomerAppError::ValidationFailed { field: "quantity" });
            }
        };
        // P4: the customer side becomes the shop side only here, through `to_shop`.
        let shop_side = plan.customer_side.to_shop();
        let unavailable = |why: &str| CustomerAppError::RateUnavailable(why.to_owned());
        let line = self
            .published_rates
            .line_for(
                tx,
                branch,
                LineSelector {
                    code: plan.currency,
                    side: shop_side,
                    group: GroupCode::all(),
                    quantity: Some(quantity),
                },
            )
            .await
            .map_err(|e| CustomerAppError::Server(e.to_string()))?
            .ok_or_else(|| unavailable("no published line"))?;
        let thb = ExchangeArithmetic::thb(quantity, line.rate, line.unit_size);
        if line.stale {
            let policy = self
                .published_rates
                .stale_policy_in(tx, &line)
                .await
                .map_err(|e| CustomerAppError::Server(e.to_string()))?;
            policy
                .admits(&line, thb)
                .map_err(|_| unavailable("stale line refused"))?;
        }
        Ok(PlannerEstimate {
            thb,
            rate: line.rate,
            unit_size: line.unit_size,
            stale: line.stale,
            source_date: line.source_date,
        })
    }
}
