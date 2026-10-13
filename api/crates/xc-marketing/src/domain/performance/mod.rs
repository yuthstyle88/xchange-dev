//! MK-7 (#1406): campaign performance (design `marketing-campaigns.md` D-15, §6, §6.1).
//!
//! - [`promo_expense::PromoExpenseFacts`]: the REWARD expense executive's `contribution_thb` needs (ruling 3 on #1406).
//! - [`commission::CampaignCommission`]: the commission term of attributed contribution; today Broker is not
//!   connected, so it is unavailable (`awaitsBroker`).
//! - [`contribution::attributed_contribution`]: realized margin − REWARD expense − commission (D-15, ruling 4).
//! - [`version_performance::VersionPerformance`]: one M04 row, per version and period.

pub mod commission;
pub mod contribution;
pub mod period;
pub mod promo_expense;
pub mod unavailable;
pub mod version_performance;
