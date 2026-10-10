//! What the rest of the shop says about a branch's counter.

use xc_kernel::money::{CurrencyCode, MinorUnit, Rate, Side, UnitSize};

use crate::domain::kiosk::value_objects::counter_facts::{CounterFacts, CounterLine, LineId};
use crate::domain::kiosk::value_objects::kiosk_currencies::KioskCurrencies;

/// The counter rate of one currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterRate {
    pub code: CurrencyCode,
    pub buy: Rate,
    pub sell: Rate,
    pub unit_size: UnitSize,
    pub minor_unit: MinorUnit,
    pub line_id: LineId,
}

/// A branch as the kiosk needs to see it: its name, whether a shift is open,
/// and the counter rates now, in the order the rate table lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterView {
    pub branch_name: String,
    pub open: bool,
    pub rates: Vec<CounterRate>,
}

impl CounterView {
    /// The rates of the currencies the kiosk offers, in this view's order.
    pub fn offered(&self, currencies: &KioskCurrencies) -> Vec<CounterRate> {
        self.rates
            .iter()
            .filter(|rate| currencies.offers(rate.code))
            .copied()
            .collect()
    }

    /// What the machine needs to judge a quote or a confirm of `currency` on `side`.
    pub fn facts(
        &self,
        currencies: &KioskCurrencies,
        currency: CurrencyCode,
        side: Side,
    ) -> CounterFacts {
        let line = self
            .rates
            .iter()
            .find(|rate| rate.code == currency && currencies.offers(rate.code))
            .map(|rate| CounterLine {
                line_id: rate.line_id,
                rate: match side {
                    Side::Buy => rate.buy,
                    Side::Sell => rate.sell,
                },
                unit_size: rate.unit_size,
                minor_unit: rate.minor_unit,
            });
        CounterFacts {
            open: self.open,
            line,
        }
    }

    /// The facts when no currency or side is known: only whether the counter is open.
    pub fn facts_without_rate(&self) -> CounterFacts {
        CounterFacts {
            open: self.open,
            line: None,
        }
    }
}
