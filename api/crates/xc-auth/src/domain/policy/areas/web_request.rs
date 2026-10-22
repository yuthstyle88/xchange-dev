//! Kiosk web queue (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D2, D3, D8): the
//! signed-in member's own web request. `CustomerSelf`, as `queueTicketTake` (`areas/queue.rs`);
//! each command answers only for the member's own request.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const fn customer_self(action: &'static str) -> AreaRow {
    AreaRow {
        path: EntryPath::Rpc,
        action,
        need: Need::Permission(Permission::CustomerSelf),
    }
}

pub const ROWS: &[AreaRow] = &[
    customer_self("webRequestCreate"),
    customer_self("webRequestQrReissue"),
    customer_self("webRequestCancel"),
];
