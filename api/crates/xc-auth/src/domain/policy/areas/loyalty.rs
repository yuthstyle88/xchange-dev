//! Permission rows for the loyalty views (LY-5 #1408, design `membership-points.md` §4.6, ruling Q2).

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const CUSTOMER_TIER: &[Permission] = &[Permission::ExchangeTransact, Permission::MarketingMembers];

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "my_membership",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "customer_tier",
        need: Need::AnyOf(CUSTOMER_TIER),
    },
];
