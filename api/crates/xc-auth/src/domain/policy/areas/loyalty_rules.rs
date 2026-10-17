//! LY-6 (#1411, design `membership-points.md` §3, §4.6): permission rows for loyalty rule versions.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "loyaltyRuleDraft",
        need: Need::Permission(Permission::LoyaltyRulesManage),
    },
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "loyaltyRuleSubmit",
        need: Need::Permission(Permission::LoyaltyRulesManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "loyaltyRuleDraft",
        need: Need::Permission(Permission::LoyaltyRulesManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "loyaltyRuleSubmit",
        need: Need::Permission(Permission::LoyaltyRulesManage),
    },
];
