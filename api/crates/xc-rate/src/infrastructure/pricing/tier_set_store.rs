//! `PgTierSetStore`: `pricing_tier_sets` (pricing-tiers.md §15.2). `tiers` is jsonb `[{tier,buy,sell}]` with decimal
//! strings; `anchor` is jsonb `{source,buy,sell,fetched_at,batch_id}`.
//!
//! SKELETON (XC-V9-PRICING): every body is PV-a's. Signatures stay.

use sqlx::PgConnection;
use uuid::Uuid;
use xc_kernel::money::CurrencyCode;

use crate::domain::pricing::tier_set::{PricingTierSet, StoreFuture, TierSetStore};

pub struct PgTierSetStore;

impl TierSetStore for PgTierSetStore {
    fn insert_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        set: &'a PricingTierSet,
    ) -> StoreFuture<'a, bool> {
        let _ = (conn, set);
        Box::pin(async move { todo!("PV-a: PgTierSetStore::insert_in") })
    }

    fn latest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<PricingTierSet>> {
        let _ = (conn, branch, currency);
        Box::pin(async move { todo!("PV-a: PgTierSetStore::latest_in") })
    }

    fn latest_follow_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<PricingTierSet>> {
        let _ = (conn, branch, currency);
        Box::pin(async move { todo!("PV-a: PgTierSetStore::latest_follow_in") })
    }

    fn get_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        id: Uuid,
    ) -> StoreFuture<'a, Option<PricingTierSet>> {
        let _ = (conn, id);
        Box::pin(async move { todo!("PV-a: PgTierSetStore::get_in") })
    }

    fn by_request_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        request_id: &'a str,
    ) -> StoreFuture<'a, Option<PricingTierSet>> {
        let _ = (conn, request_id);
        Box::pin(async move { todo!("PV-a: PgTierSetStore::by_request_in") })
    }
}
