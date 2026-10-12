//! `xc-booking` — the booking bounded context: time slots with atomic capacity, then the booking itself
//! (design `docs/design/blueprint/booking-slots.md`, #555).
//!
//! Up-front scaffolding (BK-2 #1379): the types and signatures of design §5 for the slot rows, with `todo!()`
//! bodies. The implementer fills the bodies; the provided tests in `tests/` pin them.
//!
//! The modules are declared inline here so BK-2 needs no `mod.rs` outside its file scope. Where BK-3, BK-4 and
//! BK-5 add their `pub mod` lines is the lane controller's ruling (#1379 §7); the paths below stay the same either way.

pub mod domain {
    pub mod policy;
    pub mod slot;
}

pub mod application {
    pub mod slot_service;
}

pub mod infrastructure {
    pub mod slot_repository;
}
