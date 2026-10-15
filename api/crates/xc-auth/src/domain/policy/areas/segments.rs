//! MK-6 (#1404): audience segments policy rows (ruling Q1, issuecomment-6121204241).

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "segmentDefine",
        need: Need::Permission(Permission::MarketingMembers),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "segmentSnapshot",
        need: Need::Permission(Permission::MarketingMembers),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "segment_preview",
        need: Need::Permission(Permission::MarketingMembers),
    },
];

