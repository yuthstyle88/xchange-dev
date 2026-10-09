//! Currency unit size and minor unit decimal places.

use super::currency_code::CurrencyCode;
use super::minor_unit::MinorUnit;
use super::unit_size::UnitSize;

/// The unit size and minor unit precision of a currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CurrencyUnits {
    code: CurrencyCode,
    unit_size: UnitSize,
    minor_unit: MinorUnit,
}

impl CurrencyUnits {
    pub fn new(code: CurrencyCode, unit_size: UnitSize, minor_unit: MinorUnit) -> Self {
        Self {
            code,
            unit_size,
            minor_unit,
        }
    }

    pub fn code(&self) -> CurrencyCode {
        self.code
    }

    pub fn unit_size(&self) -> UnitSize {
        self.unit_size
    }

    pub fn minor_unit(&self) -> MinorUnit {
        self.minor_unit
    }
}
