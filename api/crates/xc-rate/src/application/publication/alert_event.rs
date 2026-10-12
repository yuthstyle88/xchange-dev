//! The Rate alerts publication event (design `docs/design/blueprint/rate-alerts.md` §5, #1008 AL-3).
//! AL-4's handler delivers the same static.

use xc_outbox::domain::event_kind::EventKind;

pub static RATE_ALERTS_PUBLICATION: EventKind = EventKind {
    event_type: "rate_alerts.publication",
    destination: "rate-alerts",
    aggregate_type: "branch",
};
