//! The save check (pricing-formula.md §4, §6a): before a save that can change a price is written, price every
//! affected (branch, currency, tier) from today's reference through the formula that would apply, and refuse
//! buy > sell, a price ≤ 0, an evaluation error, or a price outside the currency guard.

use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::formula::error::FormulaError;
use crate::domain::formula::price_formula::PriceFormula;
use crate::domain::policy::currency_tier_override::CurrencyTierOverride;
use crate::domain::policy::policy_tiers::PolicyTiers;
use crate::domain::policy::rate_policy::RatePolicy;

/// What would be written.
#[derive(Debug, Clone)]
pub enum SpreadCandidate {
    /// A new organisation formula: checked at every branch of the organisation whose current policy version carries
    /// no formula of its own (B7, B8).
    Organisation {
        organization_id: Uuid,
        formula: PriceFormula,
    },
    /// A branch's next policy version, as `PolicyVersionWriter` would write it.
    Branch {
        branch: BranchId,
        version: BranchCandidate,
    },
}

#[derive(Debug, Clone)]
pub struct BranchCandidate {
    /// `Some` = the version's own formula; `None` = it follows the organisation (resolved as §3).
    pub formula: Option<PriceFormula>,
    /// The percentages and guards (B18).
    pub policy: RatePolicy,
    /// The value tiers when the switch is on, else `None` (single-price line only).
    pub tiers: Option<PolicyTiers>,
    /// Every per-currency override of the version after the change.
    pub overrides: Vec<CurrencyTierOverride>,
}

/// AUTO = the draft path (§2 bindings); FOLLOW = the §3a binding on the market anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadMode {
    Auto,
    Follow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpreadReason {
    BuyAboveSell,
    Formula(FormulaError),
    /// The price's distance from the reference, `(price ÷ ref − 1) × 100`, is outside the currency guard (B18).
    OutsideGuard {
        side: Side,
        pct: Decimal,
        min: Decimal,
        max: Decimal,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpreadFailure {
    pub branch: BranchId,
    pub currency: CurrencyCode,
    /// `None` = the single-price line; `Some(1..=4)` = that tier.
    pub tier: Option<u8>,
    pub mode: SpreadMode,
    pub buy: Option<Decimal>,
    pub sell: Option<Decimal>,
    pub reason: SpreadReason,
}

impl SpreadFailure {
    /// One line of the save refusal's message (§4 step 3), e.g. `MAIN USD buy 33.8837374 > sell 33.7293303`,
    /// `MAIN USD tier 3 FOLLOW buy 1.1 > sell 1.0`, `B2 JPY tier 1 divideByZero`,
    /// `MAIN USD buy 34.1 outside guard [-1, 1] (+1.2 %)`. Prices are printed normalised (no trailing zeros).
    pub fn describe(&self) -> String {
        let mut head = format!("{} {}", self.branch.as_str(), self.currency.as_str());
        if let Some(n) = self.tier {
            head.push_str(&format!(" tier {n}"));
        }
        if self.mode == SpreadMode::Follow {
            head.push_str(" FOLLOW");
        }
        let show = |v: Option<Decimal>| v.map_or("-".to_owned(), |d| d.normalize().to_string());
        match &self.reason {
            SpreadReason::BuyAboveSell => {
                format!("{head} buy {} > sell {}", show(self.buy), show(self.sell))
            }
            SpreadReason::Formula(e) => format!("{head} {}", e.code()),
            SpreadReason::OutsideGuard {
                side,
                pct,
                min,
                max,
            } => {
                let (name, price) = match side {
                    Side::Buy => ("buy", self.buy),
                    Side::Sell => ("sell", self.sell),
                };
                format!(
                    "{head} {name} {} outside guard [{}, {}] ({:+} %)",
                    show(price),
                    min.normalize(),
                    max.normalize(),
                    pct.normalize()
                )
            }
        }
    }
}

/// One priced row, failing or not: the preview's table (§4, B17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpreadRow {
    pub branch: BranchId,
    pub currency: CurrencyCode,
    pub tier: Option<u8>,
    pub mode: SpreadMode,
    pub buy: Option<Decimal>,
    pub sell: Option<Decimal>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpreadReport {
    /// Every row priced, in branch, currency, tier order.
    pub priced: Vec<SpreadRow>,
    pub failures: Vec<SpreadFailure>,
    /// Currencies with no current reference line from their anchor: not checked, never silently passed (§4 step 4).
    pub not_checked: Vec<(BranchId, CurrencyCode)>,
}

impl SpreadReport {
    pub fn is_ok(&self) -> bool {
        self.failures.is_empty()
    }
}

pub struct SpreadCheck;

impl SpreadCheck {
    /// §4 steps 1–4, in the caller's transaction and before it writes. First takes
    /// `SELECT id FROM organizations WHERE id = $1 FOR UPDATE` on the organisation concerned (§6a "Two saves at
    /// once", B19), then prices every affected branch and currency. Failures are listed in branch, currency, tier
    /// order; an `Err` is only a database failure.
    pub async fn run(
        tx: &mut Transaction<'_, Postgres>,
        candidate: &SpreadCandidate,
    ) -> Result<SpreadReport, RateEngineError> {
        let _ = (tx, candidate);
        todo!("pricing-formula wave 2: SpreadCheck::run")
    }
}
