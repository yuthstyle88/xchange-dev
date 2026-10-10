//! The Broker attribution event (design §5.3). RA-2's handler delivers the same static.

use xc_outbox::domain::event_kind::EventKind;

pub static BROKER_ATTRIBUTION: EventKind = EventKind {
    event_type: "broker.attribution",
    destination: "broker",
    aggregate_type: "referral_attribution",
};
