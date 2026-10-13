//! `xc-verification` — verification cases a reviewer decides, with documents kept in `DocumentStore`
//! (design `docs/design/blueprint/verification.md`, #557).
//!
//! Up-front scaffolding (VF-3 #1389): the types and signatures of design §5 for the customer side, with `todo!()`
//! bodies. The implementer fills the bodies; the provided tests in `tests/case_service_test.rs` pin them.
//!
//! The modules are declared inline here so VF-3 needs no `application/mod.rs` or `infrastructure/mod.rs`, which
//! design §9 lists in no ticket. Where VF-4, VF-5, VF-6 and VF-12 add their `pub mod` lines is the lane
//! controller's ruling (#1389 §5); the paths below stay the same either way.

pub mod application;
pub mod domain;
pub mod infrastructure;
