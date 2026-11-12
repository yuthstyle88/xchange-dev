//! Policy row for `pricingRulesSave` (xchange#6555, `docs/design/blueprint/owner-set-values.md` §2): the Owner's
//! organisation-wide pricing rules. The `pricing_rules` view reads through `exchangeSettingsRead` (`SettingsRead`).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::{AreaRow, RowScope};
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "pricingRulesSave",
    need: Need::Permission(Permission::PricingRulesSave),
    scope: RowScope::Organisation,
}];
