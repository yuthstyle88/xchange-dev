//! Machine routes outside the session model (FD-3 #974, design `docs/design/blueprint/broker-seam.md`
//! §6.4 and §7): each checks its own system token; there is no `EntryPath` and no policy row
//! (the owner's decision, 2026-10-10, #563).

pub mod broker_exchanges;
pub mod token;
