//! `LoyaltyEventKinds` (design §6, D-4): the two loyalty events, declared once as statics.
//!
//! SCAFFOLD (LY-1 up-front tests): the values below are placeholders; the implementer sets them.

use xc_outbox::domain::event_kind::EventKind;

/// `loyalty.exchange_completed`, aggregate `transaction` (D-4).
pub static LOYALTY_COMPLETED: EventKind = EventKind {
    event_type: "loyalty.exchange_completed",
    destination: "loyalty",
    aggregate_type: "transaction",
};

/// `loyalty.exchange_reversed`, aggregate `transaction` (D-4).
pub static LOYALTY_REVERSED: EventKind = EventKind {
    event_type: "loyalty.exchange_reversed",
    destination: "loyalty",
    aggregate_type: "transaction",
};
