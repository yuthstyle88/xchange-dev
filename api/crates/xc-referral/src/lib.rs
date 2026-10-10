//! `xc-referral` — v3's half of the Broker seam (design `docs/design/blueprint/broker-seam.md`, #563).
//!
//! FD-1 creates the crate with the exchange feed (§6). The attribution capture (§4, the referral
//! capture ticket) adds its own modules beside these.

pub mod application;
pub mod domain;
pub mod infrastructure;
