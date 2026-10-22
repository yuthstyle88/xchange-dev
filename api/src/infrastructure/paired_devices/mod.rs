//! The one pairing core for every paired device (design `kiosk-shared-phone.md` §7.3 T-D1, T-D2;
//! §7.5): a short-lived code, a revocable token per device, both stored only as SHA-256.
//!
//! The rules are the drawer agent's (`device_agents`, design `xchange-v3.md` §5.4c), unchanged;
//! each kind keeps its own table and implements [`store::PairedDeviceStore`] for it:
//! - `device_agents::pg_agent_repository::PgAgentRepository` — the counter drawer agent (BT-2);
//! - `branch_tablets::pg_tablet_store::PgBranchTabletStore` — the branch tablet (BT-3).
//!
//! | File | Unit | Ticket |
//! |---|---|---|
//! | `kind.rs` | [`kind::DeviceKind`] | BT-0 (filled: constants only) |
//! | `store.rs` | [`store::PairedDeviceStore`], [`store::PairedDevice`], [`store::NewPaired`] | BT-0 |
//! | `secret.rs` | [`secret::DeviceSecret`] | BT-1 |
//! | `pairing.rs` | [`pairing::DevicePairing`] and its outcomes | BT-1 |

pub mod kind;
pub mod pairing;
pub mod secret;
pub mod store;

/// A pairing code is valid this many minutes after it is issued (T-D2).
pub const CODE_VALID_MINUTES: i64 = 10;

/// Pair attempts allowed per address and 15-minute window; the next one is `RateLimited` (T-D2).
pub const PAIR_ATTEMPTS_PER_WINDOW: i32 = 10;

/// Failed token checks allowed per address and 15-minute window; the next one is `RateLimited`
/// (`agent_service.rs:114-117`, §7.7 `me`).
pub const TOKEN_FAILURES_PER_WINDOW: i32 = 30;

/// The attempt window, in minutes.
pub const ATTEMPT_WINDOW_MINUTES: i64 = 15;
