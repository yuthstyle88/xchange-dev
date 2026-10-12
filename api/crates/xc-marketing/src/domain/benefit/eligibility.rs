//! `Eligibility` and skip reasons (design §4.3, §6, §7).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use xc_kernel::money::Side;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    NotInSegment,
    TierMismatch,
    BudgetExhausted,
    CapReached,
    CouponInvalid,
    ChannelMismatch,
    CurrencyMismatch,
    SideMismatch,
    MinThbNotMet,
    BranchMismatch,
    NotReferred,
}

impl SkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkipReason::NotInSegment => "notInSegment",
            SkipReason::TierMismatch => "tierMismatch",
            SkipReason::BudgetExhausted => "budgetExhausted",
            SkipReason::CapReached => "capReached",
            SkipReason::CouponInvalid => "couponInvalid",
            SkipReason::ChannelMismatch => "channelMismatch",
            SkipReason::CurrencyMismatch => "currencyMismatch",
            SkipReason::SideMismatch => "sideMismatch",
            SkipReason::MinThbNotMet => "minThbNotMet",
            SkipReason::BranchMismatch => "branchMismatch",
            SkipReason::NotReferred => "notReferred",
        }
    }
}

/// Evaluation context provided to `Eligibility::check`.
#[derive(Debug, Clone, Default)]
pub struct EligibilityContext<'a> {
    pub branch: Option<&'a str>,
    pub channel: Option<&'a str>,
    pub customer_id: Option<&'a str>,
    pub tier: Option<&'a str>,
    pub currency: Option<&'a str>,
    pub shop_side: Option<Side>,
    pub thb_amount: Option<Decimal>,
    pub in_segment: bool,
    pub marketing_consent: bool,
    pub is_referred: bool,
    pub has_coupon: bool,
}

mod option_side_serde {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<Side>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(Side::Buy) => serializer.serialize_some("BUY"),
            Some(Side::Sell) => serializer.serialize_some("SELL"),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Side>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt.as_deref() {
            Some("BUY") => Ok(Some(Side::Buy)),
            Some("SELL") => Ok(Some(Side::Sell)),
            Some(other) => Err(serde::de::Error::custom(format!("unknown side: {other}"))),
            None => Ok(None),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Eligibility {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branches: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currencies: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_side_serde"
    )]
    pub shop_side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_thb: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tiers: Option<Vec<String>>,
    #[serde(default)]
    pub segment: bool,
    #[serde(default)]
    pub referred: bool,
    #[serde(default)]
    pub requires_coupon: bool,
}

impl Eligibility {
    pub fn check(&self, ctx: &EligibilityContext<'_>) -> Result<(), SkipReason> {
        // 1. Branches
        if let Some(ref branches) = self.branches {
            match ctx.branch {
                Some(b) if branches.iter().any(|br| br == b) => {}
                _ => return Err(SkipReason::BranchMismatch),
            }
        }

        // 2. Channels
        if let Some(ref channels) = self.channels {
            match ctx.channel {
                Some(ch) if channels.iter().any(|c| c == ch) => {}
                _ => return Err(SkipReason::ChannelMismatch),
            }
        }

        // 3. Currencies
        if let Some(ref currencies) = self.currencies {
            match ctx.currency {
                Some(curr) if currencies.iter().any(|c| c == curr) => {}
                _ => return Err(SkipReason::CurrencyMismatch),
            }
        }

        // 4. Shop side
        if let Some(side) = self.shop_side {
            match ctx.shop_side {
                Some(s) if s == side => {}
                _ => return Err(SkipReason::SideMismatch),
            }
        }

        // 5. Min THB
        if let Some(min) = self.min_thb {
            match ctx.thb_amount {
                Some(amount) if amount >= min => {}
                _ => return Err(SkipReason::MinThbNotMet),
            }
        }

        // 6. Requires coupon
        if self.requires_coupon && !ctx.has_coupon {
            return Err(SkipReason::CouponInvalid);
        }

        // 7. Tiers
        if let Some(ref tiers) = self.tiers {
            if ctx.customer_id.is_none() {
                return Err(SkipReason::TierMismatch);
            }
            match ctx.tier {
                Some(t) if tiers.iter().any(|tier| tier == t) => {}
                _ => return Err(SkipReason::TierMismatch),
            }
        }

        // 8. Segment targeting
        if self.segment && (ctx.customer_id.is_none() || !ctx.in_segment || !ctx.marketing_consent)
        {
            return Err(SkipReason::NotInSegment);
        }

        // 9. Referred
        if self.referred && !ctx.is_referred {
            return Err(SkipReason::NotReferred);
        }

        Ok(())
    }
}
