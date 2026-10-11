//! Rate repository trait.
//!
//! Since RE-7 #604 (rates.md §8 migration 6, §9) there is no `rates` table and no direct save:
//! a branch's counter rates are its current publication's lines, read here per 1 unit for the
//! v2 shapes that print rates per 1 (the snapshot, the desk config, the kiosk's `manage`).

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Rate;

/// One currency of a branch's current publication, per 1 unit.
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentRateRow {
    pub code: String,
    /// `buy_rate / unit_size`; `None` when the side is not published.
    pub buy: Option<Rate>,
    /// `sell_rate / unit_size`; `None` when the side is not published.
    pub sell: Option<Rate>,
    /// The publication's `published_at`: one instant for every line.
    pub updated_at: DateTime<Utc>,
    /// `source_snapshot.partner.name`, `""` when the line has no partner.
    pub partner_name: String,
    /// `source_snapshot.partner.cost`, `None` when the line has no partner or no cost.
    pub partner_cost: Option<Rate>,
}

pub trait RateRepository: Send + Sync {
    fn list_currencies(
        &self,
    ) -> impl Future<Output = AppResult<Vec<(String, String, Option<String>)>>> + Send;

    /// The branch's current publication, one row per currency (the `ALL` group; of a tiered
    /// currency, its first tier, as every list shows it). Empty when the branch has none.
    fn current_rates(
        &self,
        branch: &BranchId,
    ) -> impl Future<Output = AppResult<Vec<CurrentRateRow>>> + Send;
}
