//! #5558 (design `docs/design/org-wide-actor-branch.md` §12): rate drafts, rate policies and the market feed, moved from `Policy::need_table`; the organisation-level pricing routes get their own rows in place of the borrowed `rateDraft` / `ratePolicySave`.

use super::{AreaRow, RowScope};
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const fn row(path: EntryPath, action: &'static str, need: Need) -> AreaRow {
    AreaRow {
        path,
        action,
        need,
        scope: RowScope::Request,
    }
}

const fn p(permission: Permission) -> Need {
    Need::Permission(permission)
}

pub const ROWS: &[AreaRow] = &[
    row(EntryPath::Rpc, "rateDraft", p(Permission::RatesDraft)),
    row(EntryPath::StaffRpc, "rateDraft", p(Permission::RatesDraft)),
    row(
        EntryPath::Rpc,
        "ratePolicySave",
        p(Permission::RatePolicySave),
    ),
    row(
        EntryPath::Rpc,
        "marketRatesRefresh",
        p(Permission::MarketRatesRefresh),
    ),
    row(
        EntryPath::Rpc,
        "rateReferenceFetch",
        p(Permission::RatesDraft),
    ),
    row(EntryPath::Rpc, "pricing_formula", p(Permission::RatesDraft)),
    row(
        EntryPath::Rpc,
        "pricingFormulaSave",
        p(Permission::RatePolicySave),
    ),
    row(
        EntryPath::Rpc,
        "pricingModeSave",
        p(Permission::RatePolicySave),
    ),
    row(
        EntryPath::Rpc,
        "denominationGroupCreate",
        p(Permission::RatesDraft),
    ),
    row(
        EntryPath::Rpc,
        "denominationGroupMap",
        p(Permission::RatesDraft),
    ),
    row(
        EntryPath::Rpc,
        "denominationGroupRemove",
        p(Permission::RatesDraft),
    ),
    row(
        EntryPath::Rpc,
        "admin_denomination_groups",
        p(Permission::RatesDraft),
    ),
];
