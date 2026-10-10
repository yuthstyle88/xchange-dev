//! `CallOrder` (design §5, Q6, owner Q-A2): the order "call next" takes waiting tickets in.
//! BOOKING before WALK_IN; within a line, the lowest sequence first.

use std::cmp::Ordering;

use super::queue_ticket::QueueTicket;
use super::service_type::ServiceType;

/// The calling order of waiting tickets.
pub struct CallOrder;

impl CallOrder {
    /// The rank of a line: BOOKING is called before WALK_IN (owner Q-A2). Lower is called first.
    pub fn rank(service_type: ServiceType) -> u8 {
        match service_type {
            ServiceType::Booking => 0,
            ServiceType::WalkIn => 1,
        }
    }

    /// `Less` when `a` is called before `b`: by line rank, then by the lower sequence.
    pub fn cmp(a: &QueueTicket, b: &QueueTicket) -> Ordering {
        Self::rank(a.service_type)
            .cmp(&Self::rank(b.service_type))
            .then_with(|| a.sequence.cmp(&b.sequence))
    }
}
