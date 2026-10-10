//! Rate alert domain (design §2, §4, §5).

pub mod condition;
pub mod crossing_detector;
pub mod error;
pub mod rate_alert;
pub mod rate_alert_event;
pub mod relation;
pub mod repository;
pub mod settings;
pub mod source_policy;

/// The customer's side (`customer-app.md` P4), `xc-kernel`'s since #558 APP-2 (#1000). Callers and tests use this path.
pub use xc_kernel::money::CustomerSide;
