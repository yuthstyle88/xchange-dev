//! `xc-booking` — the booking bounded context: time slots with atomic capacity, then the booking itself
//! (design `docs/design/blueprint/booking-slots.md`, #555).
//!
//! Up-front scaffolding (BK-2 #1379, BK-3 #1380): types and signatures of design §5 with `todo!()`
//! bodies. The implementer fills the bodies; the provided tests in `tests/` pin them.

pub mod domain {
    pub mod booking;
    pub mod policy;
    pub mod slot;
}

pub mod application {
    pub mod booking_service;
    pub mod check_in;
    pub mod lifecycle;
    pub mod slot_service;
    pub mod sweeper;
}

pub mod infrastructure {
    pub mod booking_repository;
    pub mod slot_repository;
}
