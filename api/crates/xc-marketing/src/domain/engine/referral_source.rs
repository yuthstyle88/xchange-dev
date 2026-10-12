//! `ReferralSource` source trait (design §6, §7, D-12).

use chrono::{DateTime, Utc};

pub trait ReferralSource: Send + Sync {
    fn referred(&self, customer: &str, at: DateTime<Utc>) -> bool;
}
