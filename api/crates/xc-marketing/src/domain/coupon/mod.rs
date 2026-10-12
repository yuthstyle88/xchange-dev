//! Coupons and the benefit refusals (design D-8, D-9, §4.1 `coupons`; MK-3 #1395).

pub mod benefit_error;
pub mod code_hasher;
#[allow(clippy::module_inception)]
pub mod coupon;
