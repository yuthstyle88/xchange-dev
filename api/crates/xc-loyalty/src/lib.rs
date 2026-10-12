//! `xc-loyalty` — xchange as a client of the central Loyalty service (design
//! `docs/design/blueprint/membership-points.md` §6, #559 LY-1).
//!
//! v3 holds no points and no ledger (D-1). This crate owns the two outbound events, the customer
//! reference sent to Loyalty, the facts and the wire body. It depends on `xc-outbox` and
//! `xc-kernel`, never on `xc-exchange` (design §6).

pub mod application;
pub mod domain;
pub mod infrastructure;
