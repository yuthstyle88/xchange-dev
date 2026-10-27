//! `PgFollowResultStore`: `pricing_follow_results` (pricing-tiers.md §15.2, §15.5).
//!
//! SKELETON (XC-V9-PRICING): every body is PV-f2a's. Signatures stay.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use xc_kernel::money::CurrencyCode;

use crate::domain::pricing::follow_plan::{FollowResult, FollowResultStore};
use crate::domain::pricing::tier_set::StoreFuture;

pub struct PgFollowResultStore;

impl FollowResultStore for PgFollowResultStore {
    fn record_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        result: &'a FollowResult,
    ) -> StoreFuture<'a, ()> {
        let _ = (conn, result);
        Box::pin(async move { todo!("PV-f2a: PgFollowResultStore::record_in") })
    }

    fn latest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<FollowResult>> {
        let _ = (conn, currency);
        Box::pin(async move { todo!("PV-f2a: PgFollowResultStore::latest_in") })
    }

    fn newest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
    ) -> StoreFuture<'a, Option<DateTime<Utc>>> {
        let _ = (conn,);
        Box::pin(async move { todo!("PV-f2a: PgFollowResultStore::newest_in") })
    }
}
