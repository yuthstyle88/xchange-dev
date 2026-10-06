//! Currency code in v2's stock register.

use std::fmt;

/// An unrecognized currency code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unknown currency code")]
pub struct UnknownCurrency;

const CODES: [&str; 153] = [
    "THB", "USD", "EUR", "GBP", "JPY", "CNY", "AUD", "AED", "AFN", "ALL", "AMD", "AOA", "ARS",
    "AWG", "AZN", "BAM", "BBD", "BDT", "BHD", "BIF", "BMD", "BND", "BOB", "BRL", "BSD", "BTN",
    "BWP", "BYN", "BZD", "CAD", "CDF", "CHF", "CLP", "COP", "CRC", "CUP", "CVE", "CZK", "DJF",
    "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "FJD", "FKP", "GEL", "GHS", "GIP", "GMD", "GNF",
    "GTQ", "GYD", "HKD", "HNL", "HTG", "HUF", "IDR", "ILS", "INR", "IQD", "IRR", "ISK", "JMD",
    "JOD", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD", "KZT", "LAK", "LBP", "LKR",
    "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP", "MRU", "MUR", "MVR",
    "MWK", "MXN", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR", "NZD", "OMR", "PAB", "PEN",
    "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD", "RUB", "RWF", "SAR", "SBD", "SCR",
    "SDG", "SEK", "SGD", "SHP", "SLE", "SOS", "SRD", "SSP", "STN", "SYP", "SZL", "TJS", "TMT",
    "TND", "TOP", "TRY", "TTD", "TWD", "TZS", "UAH", "UGX", "UYU", "UZS", "VES", "VND", "VUV",
    "WST", "XAF", "XCD", "XCG", "XOF", "XPF", "YER", "ZAR", "ZMW", "ZWG",
];

const ALL_CODES: [CurrencyCode; 153] = {
    let mut arr = [CurrencyCode(0); 153];
    let mut i = 0;
    while i < 153 {
        arr[i] = CurrencyCode(i as u8);
        i += 1;
    }
    arr
};

/// One of the 153 currency codes in the stock register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CurrencyCode(u8);

impl CurrencyCode {
    /// Thai Baht (the base currency).
    pub const THB: CurrencyCode = CurrencyCode(0);

    /// Parses a 3-letter currency code (exact, case-sensitive).
    pub fn parse(text: &str) -> Result<CurrencyCode, UnknownCurrency> {
        for (i, &code) in CODES.iter().enumerate() {
            if code == text {
                return Ok(CurrencyCode(i as u8));
            }
        }
        Err(UnknownCurrency)
    }

    /// The 3-letter code as a static string slice.
    pub fn as_str(self) -> &'static str {
        CODES[self.0 as usize]
    }

    /// All registered currency codes in v2's order with `THB` first.
    pub fn all() -> &'static [CurrencyCode] {
        &ALL_CODES
    }

    /// True if this currency is Thai Baht.
    pub fn is_thb(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
