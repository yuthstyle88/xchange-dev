//! `BindingQuotePolicy` (design `docs/design/blueprint/binding-quote.md` D-10, D-14, D-15, D-17): the rules an
//! organisation's binding quote is issued under. Pure: the caller reads the inputs.

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::money::Money;

use super::quote::QuoteChannel;

/// D-10: no lifetime entered ⇒ binding quotes are off.
pub const NOT_ENABLED: &str = "validationFailed: binding quotes are not enabled";
/// D-14: a binding quote is DESK only (O-2 A). The command maps the `validationFailed:channel:` prefix to field `channel`.
pub const DESK_ONLY: &str = "validationFailed:channel: a binding quote is issued at the desk only";
/// D-17: a binding quote names a verified customer (O-8 A). The command maps the `validationFailed: customerId`
/// prefix to field `customerId`.
pub const NOT_VERIFIED: &str =
    "validationFailed: customerId must name a customer verified until after now";

/// An organisation's binding-quote settings (`exchange_settings.binding_quote_ttl_seconds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingQuotePolicy {
    /// `None` = not entered: binding quotes are off (D-10). No default (O-3 A).
    pub ttl: Option<Duration>,
}

impl BindingQuotePolicy {
    /// D-10: the lock's lifetime, or `Err(Domain(NOT_ENABLED))` when none is entered.
    pub fn enabled_ttl(&self) -> AppResult<Duration> {
        self.ttl.ok_or_else(|| AppError::Domain(NOT_ENABLED.into()))
    }

    /// D-14: `Desk` is `Ok`; `App`, `Kiosk` and `Booking` are `Err(Domain(DESK_ONLY))`.
    pub fn check_channel(&self, channel: QuoteChannel) -> AppResult<()> {
        match channel {
            QuoteChannel::Desk => Ok(()),
            QuoteChannel::App | QuoteChannel::Kiosk | QuoteChannel::Booking => {
                Err(AppError::Domain(DESK_ONLY.into()))
            }
        }
    }

    /// D-17: `verified_until` is the customer's `Readiness::Verified { until }` (`None` for no customer and for
    /// every other readiness). `Ok` only when `until > now`; else `Err(Domain(NOT_VERIFIED))`.
    pub fn check_customer(
        &self,
        verified_until: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        match verified_until {
            Some(u) if u > now => Ok(()),
            _ => Err(AppError::Domain(NOT_VERIFIED.into())),
        }
    }

    /// D-15 (T2 at issue): `true` when the quote's total `settle_thb` exceeds the branch limit; `None` = no limit.
    pub fn needs_approval(&self, settle_thb: Money, limit: Option<Money>) -> bool {
        limit.is_some_and(|l| settle_thb.minor_i128() > l.minor_i128())
    }
}
