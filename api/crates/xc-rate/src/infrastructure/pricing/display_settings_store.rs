//! `PgDisplaySettingsStore`: `rate_display_settings` (pricing-tiers.md §15.2, §15.8).
//!
//! SKELETON (XC-V9-PRICING): every body is PV-a's. Signatures stay.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;

use crate::domain::pricing::display_settings::{
    DisplayFrom, DisplaySettings, DisplaySettingsStore,
};
use crate::domain::pricing::tier_set::StoreFuture;

pub struct PgDisplaySettingsStore;

impl DisplaySettingsStore for PgDisplaySettingsStore {
    fn effective_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
    ) -> StoreFuture<'a, (DisplaySettings, DisplayFrom)> {
        let _ = (conn, branch);
        Box::pin(async move { todo!("PV-a: PgDisplaySettingsStore::effective_in") })
    }

    fn save_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        settings: &'a DisplaySettings,
        by: &'a str,
        at: DateTime<Utc>,
    ) -> StoreFuture<'a, ()> {
        let _ = (conn, branch, settings, by, at);
        Box::pin(async move { todo!("PV-a: PgDisplaySettingsStore::save_in") })
    }

    fn clear_in<'a>(&'a self, conn: &'a mut PgConnection, branch: &'a str) -> StoreFuture<'a, ()> {
        let _ = (conn, branch);
        Box::pin(async move { todo!("PV-a: PgDisplaySettingsStore::clear_in") })
    }

    fn newest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
    ) -> StoreFuture<'a, Option<DateTime<Utc>>> {
        let _ = (conn,);
        Box::pin(async move { todo!("PV-a: PgDisplaySettingsStore::newest_in") })
    }
}
