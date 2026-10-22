//! The branch's own customer tablet (design `kiosk-shared-phone.md` §7): paired on the pairing
//! core (`paired_devices`), stored in `branch_tablets`, holding at most one customer session at a
//! time, which the scan or the cancel ends on the server (T-D7).
//!
//! | File | Unit | Ticket |
//! |---|---|---|
//! | `mod.rs` | the tablet's data types | BT-0 |
//! | `pg_tablet_store.rs` | `PgBranchTabletStore` (`PairedDeviceStore`, `TabletSessions`) | BT-3 |
//! | `tablet_service.rs` | `BranchTabletService` (`me`, the sign-in binding, the staff list) | BT-3 |

pub mod pg_tablet_store;
pub mod tablet_service;

use chrono::{DateTime, Utc};

use crate::infrastructure::paired_devices::store::PairedDevice;

/// The tablet's cookie (T-D3): `__Host-xc_tablet=xcbt_<64 hex>; Path=/; HttpOnly; Secure;
/// SameSite=Strict; Max-Age=34560000`. Never readable by page script; only its SHA-256 is stored.
pub const TABLET_COOKIE: &str = "__Host-xc_tablet";

/// The cookie's `Max-Age`: 400 days, the browser cap (T-D3).
pub const TABLET_COOKIE_MAX_AGE: i64 = 34_560_000;

/// A customer session bound to a tablet expires at most this many minutes after sign-in, and is
/// never refreshed (§7.10 Q-T1 A, row T23).
pub const TABLET_SESSION_MINUTES: i64 = 30;

/// One branch tablet as stored. No hash is ever handed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchTablet {
    /// A UUID, lower case with dashes.
    pub id: String,
    pub branch_id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    /// `None` once the code is used (or never set).
    pub pair_code_expires_at: Option<DateTime<Utc>>,
    pub paired_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub user_agent: Option<String>,
    /// The user of the customer session bound to the tablet, if any (`users.id`).
    pub customer_user_id: Option<String>,
}

impl PairedDevice for BranchTablet {
    fn id(&self) -> &str {
        &self.id
    }

    fn branch_id(&self) -> &str {
        &self.branch_id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

/// What a tablet says about itself when it pairs and on every `me`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabletSeen<'a> {
    /// The request's `User-Agent`, `""` when absent.
    pub user_agent: &'a str,
}

/// `GET /api/branch-tablet/me`'s answer (T-D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabletMe {
    /// `{paired: false}`. `clear_cookie` when the request carried a tablet cookie that is not a
    /// live tablet's (revoked, unknown, malformed), so the answer also clears it.
    Unpaired { clear_cookie: bool },
    /// `{paired: true, tabletId, branchCode, name}`.
    Paired {
        tablet_id: String,
        /// The tablet's `branch_id`, the code the page puts in `?branch=`.
        branch_code: String,
        name: String,
    },
}

/// A tablet's state in the staff list (§7.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabletState {
    /// Never paired.
    Pending,
    Paired,
    Revoked,
}

impl TabletState {
    /// `PENDING`, `PAIRED` or `REVOKED`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Paired => "PAIRED",
            Self::Revoked => "REVOKED",
        }
    }
}

/// A tablet listed for the staff (`{action: "list"}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedTablet {
    pub tablet_id: String,
    pub branch_id: String,
    pub name: String,
    pub state: TabletState,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub paired_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}
