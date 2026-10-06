//! Which currencies the kiosk of a branch offers.

use xc_kernel::money::CurrencyCode;

use crate::domain::kiosk::value_objects::currency_switch::CurrencySwitch;

/// The currencies a kiosk offers when its branch has switched nothing.
pub const DEFAULT_CODES: [&str; 6] = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"];

/// The six default currencies, plus those the branch switched on, minus those it switched off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KioskCurrencies {
    offered: Vec<CurrencyCode>,
}

impl KioskCurrencies {
    pub fn new(switches: &[CurrencySwitch]) -> Self {
        let mut offered: Vec<CurrencyCode> = DEFAULT_CODES
            .iter()
            .filter_map(|code| CurrencyCode::parse(code).ok())
            .collect();
        for switch in switches {
            offered.retain(|code| *code != switch.code);
            if switch.enabled {
                offered.push(switch.code);
            }
        }
        Self { offered }
    }

    pub fn offers(&self, code: CurrencyCode) -> bool {
        self.offered.contains(&code)
    }

    /// Every currency of the register except the baht, in code order, each with
    /// whether this kiosk offers it.
    pub fn all(&self) -> Vec<CurrencySwitch> {
        let mut all: Vec<CurrencySwitch> = CurrencyCode::all()
            .iter()
            .filter(|code| !code.is_thb())
            .map(|code| CurrencySwitch {
                code: *code,
                enabled: self.offers(*code),
            })
            .collect();
        all.sort_by_key(|switch| switch.code.as_str());
        all
    }
}
