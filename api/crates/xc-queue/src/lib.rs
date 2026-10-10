//! `xc-queue` — the queue bounded context: tickets with a daily number per branch and service type
//! (design `docs/design/blueprint/queue-tickets.md`, #556).
//!
//! Up-front scaffolding (QU-A1): the types and signatures of design §5 for rows QU-1/QU-2, with
//! `todo!()` bodies. The implementer fills the bodies; the provided tests in `tests/` pin them.

pub mod application;
pub mod domain;
pub mod infrastructure;
