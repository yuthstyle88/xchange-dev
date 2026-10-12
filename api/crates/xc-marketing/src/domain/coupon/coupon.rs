//! `Coupon` (design §4.1 `coupons`, §4.2, D-9): one row per code, locked on use.
//!
//! Up-front scaffolding (MK-3 #1395): the shape the provided tests pin, with `todo!()` bodies.

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CouponStatus {
    Issued,
    Redeemed,
    Void,
}

impl CouponStatus {
    /// The `coupons.status` text: `ISSUED`, `REDEEMED` or `VOID`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Issued => "ISSUED",
            Self::Redeemed => "REDEEMED",
            Self::Void => "VOID",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ISSUED" => Some(Self::Issued),
            "REDEEMED" => Some(Self::Redeemed),
            "VOID" => Some(Self::Void),
            _ => None,
        }
    }
}

impl std::fmt::Display for CouponStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coupon {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub rule_id: Uuid,
    /// A personal coupon (D-9): set = only this customer may use it.
    pub customer_id: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub status: CouponStatus,
}
