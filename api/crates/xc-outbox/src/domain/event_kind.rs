//! `EventKind` (design §5.2): declared once per event, as a `static`, by the area that owns the
//! event. Its writer call and its handler use the same static.

/// One kind of outbound event. One business event heard by several services is one `EventKind`
/// per destination (fan-out, design §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventKind {
    /// For example `"broker.attribution"`. Unique across every registered handler.
    pub event_type: &'static str,
    /// For example `"broker"`. FIFO is per `(aggregate_type, aggregate_id, destination)`.
    pub destination: &'static str,
    /// For example `"referral_attribution"`.
    pub aggregate_type: &'static str,
}
