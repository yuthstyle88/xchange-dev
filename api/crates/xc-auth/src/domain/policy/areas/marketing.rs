//! Marketing policy rows (MK-4, design `marketing-campaigns.md` §3, §4.5, §7).

use crate::domain::access::Permission;
use crate::domain::policy::areas::AreaRow;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const SIMULATE_PERMISSIONS: &[Permission] = &[
    Permission::MarketingCampaigns,
    Permission::MarketingPerformance,
];

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "campaignCreate",
        need: Need::Permission(Permission::MarketingCampaigns),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "campaignVersionSave",
        need: Need::Permission(Permission::MarketingCampaigns),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "campaignVersionNew",
        need: Need::Permission(Permission::MarketingCampaigns),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "campaignSimulate",
        need: Need::AnyOf(SIMULATE_PERMISSIONS),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "campaignSubmit",
        need: Need::Permission(Permission::MarketingCampaigns),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "campaignPause",
        need: Need::Permission(Permission::MarketingCampaigns),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "campaignResume",
        need: Need::Permission(Permission::MarketingCampaigns),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "couponIssue",
        need: Need::Permission(Permission::MarketingCampaigns),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "marginGuardSet",
        need: Need::Permission(Permission::RatePolicySave),
    },
];
