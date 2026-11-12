//! The Owner's pricing rules (`docs/design/blueprint/owner-set-values.md` §2, §4; xchange#6555): one organisation-wide
//! set of business values, saved by the Owner as a new version each time. Every field is optional, and `None` means
//! "not set": a reader does the safe thing the design names for that field, never a number from code.
//!
//! This ticket (GA-1) only stores and validates the rules. The readers switch to them in GA-2..GA-5.

use chrono::Duration;
use rust_decimal::Decimal;

/// The largest value a `numeric(9,4)` column holds is below 10^5.
const PCT_LIMIT: i64 = 100_000;
/// The largest value a `numeric(6,2)` column holds is below 10^4.
const CASH_STEP_LIMIT: i64 = 10_000;
/// A rate is shown with at most 4 decimals (§2 row 4).
pub const MAX_RATE_DECIMALS: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PricingRules {
    /// Row 2: the limit itself, in %, with no hidden multiplier.
    pub far_from_anchor_limit_pct: Option<Decimal>,
    /// Row 3, in %.
    pub large_move_pct: Option<Decimal>,
    /// Row 4.
    pub rate_decimals: Option<RateDecimals>,
    /// Row 5, in baht. A plain decimal here; GA-4 wraps it in `xc_kernel::money::CashStep`.
    pub thb_cash_step: Option<Decimal>,
    /// Row 8: one symmetric cap, in basis points.
    pub branch_bp_cap: Option<i32>,
    /// Row 9, in %.
    pub better_pct_range: Option<PctRange>,
    /// Row 9, in %.
    pub min_spread_pct_range: Option<PctRange>,
    /// Row 16, whole minutes.
    pub market_stale_after: Option<Duration>,
    /// Row 17.
    pub advice: Option<AdviceThresholds>,
}

/// Rising `(from_price, decimals)` bands, the first from 0: a price takes the decimals of the last band it reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateDecimals {
    pub bands: Vec<(Decimal, u32)>,
}

impl RateDecimals {
    pub fn decimals_for(&self, price: Decimal) -> u32 {
        self.bands
            .iter()
            .take_while(|(from, _)| price >= *from)
            .last()
            .map_or(MAX_RATE_DECIMALS, |&(_, decimals)| decimals)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PctRange {
    pub min: Decimal,
    pub max: Decimal,
}

/// Row 17: the pricing board's advice thresholds, set together or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdviceThresholds {
    pub far_pct: Decimal,
    pub spread_ratio: Decimal,
    pub spread_gap_pt: Decimal,
    pub market_moved_pct: Decimal,
}

/// A value the Owner saved is not allowed. `field` is the API's camelCase name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricingRuleError {
    pub field: &'static str,
    pub message: String,
}

/// A reader needed a rule the Owner has not set (`pricingRuleNotSet`). `field` is the API's camelCase name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleNotSet {
    pub field: &'static str,
}

/// The rule's value, or `RuleNotSet` naming `field`.
pub fn require<T: Clone>(value: &Option<T>, field: &'static str) -> Result<T, RuleNotSet> {
    value.clone().ok_or(RuleNotSet { field })
}

fn refuse(field: &'static str, message: impl Into<String>) -> PricingRuleError {
    PricingRuleError {
        field,
        message: message.into(),
    }
}

/// Fits the column: at most `dp` decimals and below `limit` in size.
fn fits(field: &'static str, v: Decimal, dp: u32, limit: i64) -> Result<(), PricingRuleError> {
    if v.normalize().scale() > dp || v.abs() >= Decimal::from(limit) {
        return Err(refuse(
            field,
            format!("ค่าต้องมีทศนิยมไม่เกิน {dp} ตำแหน่ง และน้อยกว่า {limit}"),
        ));
    }
    Ok(())
}

fn positive(field: &'static str, v: Decimal, dp: u32, limit: i64) -> Result<(), PricingRuleError> {
    fits(field, v, dp, limit)?;
    if v <= Decimal::ZERO {
        return Err(refuse(field, "ค่าต้องมากกว่า 0"));
    }
    Ok(())
}

fn range(field: &'static str, r: &PctRange) -> Result<(), PricingRuleError> {
    fits(field, r.min, 4, PCT_LIMIT)?;
    fits(field, r.max, 4, PCT_LIMIT)?;
    if r.min > r.max {
        return Err(refuse(field, "ค่าต่ำสุดต้องไม่มากกว่าค่าสูงสุด"));
    }
    Ok(())
}

impl PricingRules {
    /// Nothing set: what an organisation reads before the Owner's first save.
    pub fn not_set() -> Self {
        Self::default()
    }

    /// The checks of design §2's "Unit and check" column, the same as the table's CHECKs.
    pub fn validate(&self) -> Result<(), PricingRuleError> {
        if let Some(v) = self.far_from_anchor_limit_pct {
            positive("farFromAnchorLimitPct", v, 4, PCT_LIMIT)?;
        }
        if let Some(v) = self.large_move_pct {
            positive("largeMovePct", v, 4, PCT_LIMIT)?;
        }
        if let Some(d) = &self.rate_decimals {
            d.validate()?;
        }
        if let Some(v) = self.thb_cash_step {
            positive("thbCashStep", v, 2, CASH_STEP_LIMIT)?;
        }
        if let Some(cap) = self.branch_bp_cap
            && cap <= 0
        {
            return Err(refuse("branchBpCap", "ค่าต้องมากกว่า 0"));
        }
        if let Some(r) = &self.better_pct_range {
            range("betterPctRange", r)?;
        }
        if let Some(r) = &self.min_spread_pct_range {
            range("minSpreadPctRange", r)?;
            if r.min < Decimal::ZERO {
                return Err(refuse("minSpreadPctRange", "ส่วนต่างขั้นต่ำต้องไม่ติดลบ"));
            }
        }
        if let Some(d) = self.market_stale_after {
            let minutes = d.num_minutes();
            if minutes <= 0 || d != Duration::minutes(minutes) || minutes > i64::from(i32::MAX) {
                return Err(refuse(
                    "marketStaleAfterMinutes",
                    "ค่าต้องเป็นจำนวนนาทีเต็มที่มากกว่า 0",
                ));
            }
        }
        if let Some(a) = &self.advice {
            for v in [
                a.far_pct,
                a.spread_ratio,
                a.spread_gap_pt,
                a.market_moved_pct,
            ] {
                positive("advice", v, 4, PCT_LIMIT)?;
            }
        }
        Ok(())
    }
}

impl RateDecimals {
    /// Rising `from_price` starting at 0, and `decimals` 0..=4.
    pub fn validate(&self) -> Result<(), PricingRuleError> {
        const FIELD: &str = "rateDecimals";
        let Some(&(first, _)) = self.bands.first() else {
            return Err(refuse(FIELD, "ต้องมีอย่างน้อยหนึ่งช่วง"));
        };
        if first != Decimal::ZERO {
            return Err(refuse(FIELD, "ช่วงแรกต้องเริ่มที่ราคา 0"));
        }
        if self.bands.windows(2).any(|w| w[1].0 <= w[0].0) {
            return Err(refuse(FIELD, "ราคาเริ่มของแต่ละช่วงต้องสูงขึ้นเรื่อย ๆ"));
        }
        if self.bands.iter().any(|&(_, d)| d > MAX_RATE_DECIMALS) {
            return Err(refuse(
                FIELD,
                format!("ทศนิยมต้องอยู่ระหว่าง 0 ถึง {MAX_RATE_DECIMALS}"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dm(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn bands(b: &[(Decimal, u32)]) -> RateDecimals {
        RateDecimals { bands: b.to_vec() }
    }

    #[test]
    fn todays_decimals_rule_reads_as_v9() {
        let d = bands(&[(dm("0"), 4), (dm("1"), 3), (dm("10"), 2)]);
        assert_eq!(d.validate(), Ok(()));
        assert_eq!(d.decimals_for(dm("0.25")), 4);
        assert_eq!(d.decimals_for(dm("1")), 3);
        assert_eq!(d.decimals_for(dm("35.1234")), 2);
    }

    #[test]
    fn decimals_bands_must_start_at_zero_and_rise() {
        assert_eq!(bands(&[]).validate().unwrap_err().field, "rateDecimals");
        assert!(bands(&[(dm("1"), 4)]).validate().is_err());
        assert!(
            bands(&[(dm("0"), 4), (dm("10"), 2), (dm("10"), 3)])
                .validate()
                .is_err()
        );
        assert!(bands(&[(dm("0"), 5)]).validate().is_err());
    }

    #[test]
    fn not_set_is_valid() {
        assert_eq!(PricingRules::not_set().validate(), Ok(()));
    }

    #[test]
    fn a_range_with_min_above_max_names_its_field() {
        let rules = PricingRules {
            better_pct_range: Some(PctRange {
                min: dm("1"),
                max: dm("-1"),
            }),
            ..PricingRules::not_set()
        };
        assert_eq!(rules.validate().unwrap_err().field, "betterPctRange");
    }

    #[test]
    fn values_must_be_positive_and_fit_their_column() {
        let zero = PricingRules {
            large_move_pct: Some(dm("0")),
            ..PricingRules::not_set()
        };
        assert_eq!(zero.validate().unwrap_err().field, "largeMovePct");
        let satang = PricingRules {
            thb_cash_step: Some(dm("0.125")),
            ..PricingRules::not_set()
        };
        assert_eq!(satang.validate().unwrap_err().field, "thbCashStep");
        let spread = PricingRules {
            min_spread_pct_range: Some(PctRange {
                min: dm("-0.1"),
                max: dm("1"),
            }),
            ..PricingRules::not_set()
        };
        assert_eq!(spread.validate().unwrap_err().field, "minSpreadPctRange");
        let stale = PricingRules {
            market_stale_after: Some(Duration::seconds(90)),
            ..PricingRules::not_set()
        };
        assert_eq!(
            stale.validate().unwrap_err().field,
            "marketStaleAfterMinutes"
        );
    }

    #[test]
    fn require_names_the_missing_field() {
        let rules = PricingRules::not_set();
        assert_eq!(
            require(&rules.thb_cash_step, "thbCashStep"),
            Err(RuleNotSet {
                field: "thbCashStep"
            })
        );
    }
}
