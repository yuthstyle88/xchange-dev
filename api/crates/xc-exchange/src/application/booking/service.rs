//! Booking service.

use std::sync::Arc;

use pos_error::AppResult;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::domain::booking::repositories::booking_ids::BookingIds;
use crate::domain::booking::repositories::booking_ports::BookingPorts;
use crate::domain::booking::repositories::booking_repository::BookingRepository;
use crate::domain::booking::repositories::member_repository::MemberRepository;

#[derive(Clone)]
pub struct BookingService {
    pub(crate) branch_tx: BranchTx,
    pub(crate) bookings: Arc<dyn BookingRepository>,
    pub(crate) members: Arc<dyn MemberRepository>,
    pub(crate) ports: Arc<dyn BookingPorts>,
    pub(crate) ids: Arc<dyn BookingIds>,
}

impl BookingService {
    pub fn new(
        branch_tx: BranchTx,
        bookings: Arc<dyn BookingRepository>,
        members: Arc<dyn MemberRepository>,
        ports: Arc<dyn BookingPorts>,
        ids: Arc<dyn BookingIds>,
    ) -> Self {
        Self {
            branch_tx,
            bookings,
            members,
            ports,
            ids,
        }
    }

    pub async fn branch_exists(&self, branch: &BranchId) -> AppResult<bool> {
        self.ports.branch_exists(branch).await
    }
}
