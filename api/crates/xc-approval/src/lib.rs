//! `xc-approval` — the approval and audit core (rates design §5, #549).
//!
//! Ticket AP-1. The modules are the class diagram's (§7); they are empty until
//! the implementation lands. The up-front tests in `tests/` name what each one
//! must provide.

pub mod application;
pub mod domain;
pub mod infrastructure;
