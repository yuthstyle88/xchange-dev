//! One currency of the kiosk, on or off.

use xc_kernel::money::CurrencyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrencySwitch {
    pub code: CurrencyCode,
    pub enabled: bool,
}
