//! `xc-outbox` — the lane's single outbox core (design `docs/design/blueprint/broker-seam.md` §5, #563 OB-1).
//!
//! The core names no destination, URL, token or event type. Each destination is one
//! `OutboxHandler` in its own area's crate; the core owns the same-transaction write,
//! idempotency, the claim and lease, per-aggregate FIFO, retry timing, expiry and redaction.

pub mod application;
pub mod domain;
pub mod infrastructure;
