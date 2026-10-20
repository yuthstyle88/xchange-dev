//! `IssueSource` (design §5, Q4): where a ticket comes from.

use xc_kernel::queue_tickets::QueueTicketSource;

use super::service_type::ServiceType;

/// Who asked for the ticket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssueSource {
    /// The kiosk's `confirm` (QUOTED → WAITING); one ticket per request.
    Kiosk { request_id: String },
    /// A booking's check-in (B02, #555); at most one active ticket per booking.
    Booking { booking_id: String },
    /// A teller at the desk, for a customer without the tablet.
    Desk,
    /// A signed-in customer in the app (S7, #3410); at most one active walk-in per branch and day.
    App { customer_id: String },
}

impl IssueSource {
    /// BOOKING for a booking, WALK_IN otherwise (Q2, Q-A2).
    pub fn service_type(&self) -> ServiceType {
        match self {
            Self::Booking { .. } => ServiceType::Booking,
            Self::Kiosk { .. } | Self::Desk | Self::App { .. } => ServiceType::WalkIn,
        }
    }
}

impl From<&QueueTicketSource> for IssueSource {
    fn from(source: &QueueTicketSource) -> Self {
        match source {
            QueueTicketSource::Kiosk { request_id } => Self::Kiosk {
                request_id: request_id.clone(),
            },
            QueueTicketSource::Booking { booking_id } => Self::Booking {
                booking_id: booking_id.clone(),
            },
        }
    }
}
