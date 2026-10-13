//! Why a performance figure has no value (design §6.1; ruling 4 on #1406). Shown as a fixed reason, never as zero.

/// The reason a figure is unavailable. The wire names match executive's `MetricUnavailable` (`awaitsLedger`,
/// `awaitsBroker`), so the executive follow-up maps them one to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// A redeeming exchange line has no `transaction_lines.realized_margin_thb` (#553 not posted for it).
    AwaitsLedger,
    /// Commission comes from Broker-Platform, which publishes nothing yet (EX-10 parked).
    AwaitsBroker,
}

impl Unavailable {
    /// The reason on the wire: `awaitsLedger` or `awaitsBroker`.
    pub fn as_str(self) -> &'static str {
        match self {
            Unavailable::AwaitsLedger => "awaitsLedger",
            Unavailable::AwaitsBroker => "awaitsBroker",
        }
    }
}
