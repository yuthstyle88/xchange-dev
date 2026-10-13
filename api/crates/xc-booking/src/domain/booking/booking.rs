//! `Booking` (design §4.1, §5): the Blueprint booking entity.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;

use super::booking_status::BookingStatus;
use super::source_channel::SourceChannel;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Booking {
    pub id: String,
    pub organization_id: Uuid,
    pub branch_id: BranchId,
    pub booking_no: String,
    pub customer_id: String,
    pub slot_id: Uuid,
    pub quote_id: Option<Uuid>,
    pub shop_side: Side,
    pub requested_currency: String,
    pub requested_quantity: Decimal,
    pub source_channel: SourceChannel,
    pub status: BookingStatus,
    pub qr_token_hash: Vec<u8>,
    pub qr_version: u32,
    pub confirm_expires_at: Option<DateTime<Utc>>,
    pub checked_in_at: Option<DateTime<Utc>>,
    pub transaction_id: Option<Uuid>,
    pub partner_attribution_id: Option<Uuid>,
    pub ended_reason: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
