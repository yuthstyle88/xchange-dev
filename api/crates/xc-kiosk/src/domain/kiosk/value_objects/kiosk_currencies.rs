//! Which currencies the kiosk of a branch offers.

use xc_kernel::money::CurrencyCode;

use crate::domain::kiosk::value_objects::currency_switch::CurrencySwitch;

/// Currencies offered by a branch's kiosk (`display = 'SHOW'`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KioskCurrencies {
    offered: Vec<CurrencyCode>,
}

impl KioskCurrencies {
    pub fn new(switches: &[CurrencySwitch]) -> Self {
        let mut offered = Vec::new();
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
