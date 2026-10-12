use rust_decimal::{Decimal, RoundingStrategy};
use xc_kernel::money::{ExchangeArithmetic, Money, Rate, Side};

use super::engine::QuoteLineInput;
use crate::domain::benefit::calculator::BenefitEffect;
use crate::domain::benefit::rule::BenefitClass;

#[derive(Debug, Clone, PartialEq)]
pub struct RateFloorResult {
    pub final_rate: Rate,
    pub fee_thb: Money,
    pub amount_thb: Money,
    pub floor_capped: bool,
    pub guard_capped: bool,
}

pub struct RateFloor;

impl RateFloor {
    pub fn apply(
        line: &QuoteLineInput,
        effect: &BenefitEffect,
        benefit_class: BenefitClass,
        max_spread_share: Option<Decimal>,
    ) -> RateFloorResult {
        match benefit_class {
            BenefitClass::Reward => RateFloorResult {
                final_rate: line.base_rate,
                fee_thb: line.fee_thb,
                amount_thb: effect.amount_thb,
                floor_capped: false,
                guard_capped: false,
            },
            BenefitClass::Price => {
                if let Some(rate) = effect.new_final_rate {
                    let uncapped = rate.value();
                    let base = line.base_rate.value();
                    let buy_floor = line.published_buy_rate.value();
                    let sell_floor = line.published_sell_rate.value();
                    let spread = (sell_floor - buy_floor).max(Decimal::ZERO);

                    let mut effective_rate = uncapped;
                    let mut floor_capped = false;
                    let mut guard_capped = false;

                    match line.shop_side {
                        Side::Buy => {
                            let floor_limit = sell_floor;
                            let guard_limit = max_spread_share.map(|share| base + (spread * share));

                            if let Some(guard) = guard_limit {
                                if effective_rate > guard {
                                    effective_rate = guard;
                                    guard_capped = true;
                                }
                            }

                            if effective_rate > floor_limit {
                                effective_rate = floor_limit;
                                floor_capped = true;
                            } else if uncapped > floor_limit && guard_limit.is_none() {
                                floor_capped = true;
                            }
                        }
                        Side::Sell => {
                            let floor_limit = buy_floor;
                            let guard_limit = max_spread_share.map(|share| base - (spread * share));

                            if let Some(guard) = guard_limit {
                                if effective_rate < guard {
                                    effective_rate = guard;
                                    guard_capped = true;
                                }
                            }

                            if effective_rate < floor_limit {
                                effective_rate = floor_limit;
                                floor_capped = true;
                            } else if uncapped < floor_limit && guard_limit.is_none() {
                                floor_capped = true;
                            }
                        }
                    }

                    let effective_rounded = effective_rate
                        .round_dp_with_strategy(8, RoundingStrategy::MidpointAwayFromZero);
                    let effective_clamped = match line.shop_side {
                        Side::Buy => effective_rounded.min(sell_floor),
                        Side::Sell => effective_rounded.max(buy_floor),
                    };

                    let final_rate =
                        Rate::parse(&effective_clamped.to_string()).unwrap_or_else(|e| {
                            panic!("effective rate {effective_clamped} must be valid Rate: {e}")
                        });
                    let thb_final =
                        ExchangeArithmetic::thb(line.quantity, final_rate, line.unit_size);
                    let thb_base =
                        ExchangeArithmetic::thb(line.quantity, line.base_rate, line.unit_size);

                    let amount_thb = match line.shop_side {
                        Side::Buy => {
                            let diff = thb_final.minor() - thb_base.minor();
                            Money::from_minor(diff.max(0)).unwrap_or(Money::ZERO)
                        }
                        Side::Sell => {
                            let diff = thb_base.minor() - thb_final.minor();
                            Money::from_minor(diff.max(0)).unwrap_or(Money::ZERO)
                        }
                    };

                    RateFloorResult {
                        final_rate,
                        fee_thb: line.fee_thb,
                        amount_thb,
                        floor_capped,
                        guard_capped,
                    }
                } else if let Some(fee) = effect.new_fee_thb {
                    let mut fee_val = fee;
                    let mut floor_capped = false;
                    let mut guard_capped = false;

                    let buy_floor = line.published_buy_rate.value();
                    let sell_floor = line.published_sell_rate.value();
                    let spread = (sell_floor - buy_floor).max(Decimal::ZERO);

                    let uncapped_discount_minor = (line.fee_thb.minor() - fee.minor()).max(0);

                    if let Some(share) = max_spread_share {
                        let raw =
                            (line.quantity.value() * (spread * share)) / line.unit_size.value();
                        let mut rounded =
                            raw.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
                        rounded.rescale(2);
                        let max_giveaway_minor = rounded.mantissa().max(0) as i64;
                        if uncapped_discount_minor > max_giveaway_minor {
                            guard_capped = true;
                            let allowed_discount_minor =
                                max_giveaway_minor.min(uncapped_discount_minor);
                            fee_val =
                                Money::from_minor(line.fee_thb.minor() - allowed_discount_minor)
                                    .unwrap_or(line.fee_thb);
                        }
                    }

                    if fee.minor() < 0 || fee_val.minor() < 0 {
                        fee_val = Money::ZERO;
                        floor_capped = true;
                    }

                    let amount_thb =
                        Money::from_minor((line.fee_thb.minor() - fee_val.minor()).max(0))
                            .unwrap_or(Money::ZERO);

                    RateFloorResult {
                        final_rate: line.base_rate,
                        fee_thb: fee_val,
                        amount_thb,
                        floor_capped,
                        guard_capped,
                    }
                } else {
                    RateFloorResult {
                        final_rate: line.base_rate,
                        fee_thb: line.fee_thb,
                        amount_thb: Money::ZERO,
                        floor_capped: false,
                        guard_capped: false,
                    }
                }
            }
        }
    }
}
