//! `TabletSessions` — ends the customer session a branch tablet holds (design
//! `kiosk-shared-phone.md` §7.3 T-D7, §7.5).
//!
//! The kiosk's scan (`issueByToken`) and the member's `webRequestCancel` call it inside their own
//! transaction, so `xc-kiosk` never names the tablet store. The root package's
//! `infrastructure::branch_tablets::pg_tablet_store::PgBranchTabletStore` implements it (BT-3);
//! the kiosk wiring installs it (BT-6).
//!
//! The default ends nothing, so code built without a tablet store behaves exactly as before
//! ([`NoTabletSessions`]).

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};

use crate::ids::BranchId;

pub trait TabletSessions: Send + Sync {
    /// Inside `tx`: the member `customer_id` (a `customers.id`, as `kiosk_requests.customer_id`
    /// holds it) is found through `customers.user_id`; the member's session bound to a tablet of
    /// `branch` (`branch_tablets.customer_session_hash`) is deleted from `user_sessions`, and that
    /// tablet's two `customer_*` columns are cleared. A session of the same member not bound to a
    /// tablet of `branch` (their own phone, a tablet of another branch) stays. Answers how many
    /// tablets were cleared. An error rolls the caller's transaction back.
    fn end_for_member_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<u64>> {
        Box::pin(async { Ok(0) })
    }
}

/// Ends nothing: the port's default, for code with no tablet store.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTabletSessions;

impl TabletSessions for NoTabletSessions {}
