//! Rule A and the drawer target, as one pure decision (xc-counter-app.md §4, §5.4, §8.5). No database: the caller
//! reads the bound counter and the branch setting, and asks here.
//!
//! - A session bound to a counter: cash is paid out there only when that counter's `cash_drawer` is on, and its
//!   drawer job targets that counter. Bound to a counter whose `cash_drawer` is off or not set: refused (rule A).
//! - A session bound to no counter (a plain browser) follows the branch setting `cashFromBrowser`: not set or on ⇒
//!   allowed, drawer job with no target (`NULL`); off ⇒ refused.
//! - A bound counter that is revoked counts as no drawer (refused); the binding is cleared on revoke anyway.
//!
//! CA-1e fills the bodies.

use super::agent_repository::DeviceAgent;

/// The refusal code of rule A `[required]`.
pub const COUNTER_NO_DRAWER: &str = "COUNTER_NO_DRAWER";
/// The refusal message of rule A `[required]`, word for word from §4.
pub const COUNTER_NO_DRAWER_MESSAGE: &str = "เคาน์เตอร์นี้ไม่มีลิ้นชักเงิน — ส่งไปชำระที่เคาน์เตอร์ที่มีลิ้นชัก";
/// The `branch_settings` name of the branch setting `[required]` (§5.4).
pub const CASH_FROM_BROWSER: &str = "cashFromBrowser";

/// What a cash payout may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CashPayout {
    /// Pay out; the drawer job targets `target_agent_id` (`None` = any drawer counter of the branch).
    Allowed { target_agent_id: Option<String> },
    /// Refuse with [`COUNTER_NO_DRAWER`].
    Refused,
}

/// The counter a session is bound to, as the server read it.
#[derive(Debug, Clone, Copy)]
pub enum Seat<'a> {
    /// A plain browser: no `user_sessions.counter_agent_id`.
    Unbound,
    /// Bound to this counter (`None` when the id no longer names a counter).
    Bound(Option<&'a DeviceAgent>),
}

/// The decision for a cash payout. `cash_from_browser` is the branch setting's value (`None` = not set = on).
pub fn cash_payout(seat: Seat<'_>, cash_from_browser: Option<bool>) -> CashPayout {
    match seat {
        Seat::Bound(Some(agent))
            if agent.revoked_at.is_none() && agent.cash_drawer == Some(true) =>
        {
            CashPayout::Allowed {
                target_agent_id: Some(agent.id.clone()),
            }
        }
        Seat::Bound(_) => CashPayout::Refused,
        Seat::Unbound => {
            if cash_from_browser == Some(false) {
                CashPayout::Refused
            } else {
                CashPayout::Allowed {
                    target_agent_id: None,
                }
            }
        }
    }
}

/// What the app's `info()` answers as `capabilities` (§8.2): each setting resolved, `None` → its safe value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub cash_drawer: bool,
    pub scanner: bool,
    pub kiosk_lock: bool,
}

impl Capabilities {
    pub fn of(agent: &DeviceAgent) -> Self {
        Self {
            cash_drawer: agent.cash_drawer == Some(true),
            scanner: agent.scanner == Some(true),
            kiosk_lock: agent.kiosk_lock != Some(false),
        }
    }
}
