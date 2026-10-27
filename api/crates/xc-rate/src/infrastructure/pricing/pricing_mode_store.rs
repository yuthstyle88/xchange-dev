//! `PgPricingModeStore`: `pricing_modes` (pricing-tiers.md §15.2, §15.5).
//!
//! SKELETON (XC-V9-PRICING): every body is PV-f2b's. Signatures stay.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use xc_kernel::money::CurrencyCode;

use crate::domain::pricing::pricing_mode::{PricingMode, PricingModeRow, PricingModeStore};
use crate::domain::pricing::tier_set::StoreFuture;

pub struct PgPricingModeStore;

impl PricingModeStore for PgPricingModeStore {
    fn save_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        currency: CurrencyCode,
        mode: PricingMode,
        by: &'a str,
        at: DateTime<Utc>,
    ) -> StoreFuture<'a, i64> {
        let _ = (conn, currency, mode, by, at);
        Box::pin(async move { todo!("PV-f2b: PgPricingModeStore::save_in") })
    }

    fn get_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<PricingModeRow>> {
        let _ = (conn, currency);
        Box::pin(async move { todo!("PV-f2b: PgPricingModeStore::get_in") })
    }

    fn following_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
    ) -> StoreFuture<'a, Vec<CurrencyCode>> {
        let _ = (conn,);
        Box::pin(async move { todo!("PV-f2b: PgPricingModeStore::following_in") })
    }

    fn newest_revision_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
    ) -> StoreFuture<'a, Option<i64>> {
        let _ = (conn,);
        Box::pin(async move { todo!("PV-f2b: PgPricingModeStore::newest_revision_in") })
    }
}
