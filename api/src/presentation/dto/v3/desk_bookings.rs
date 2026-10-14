//! DTOs for the staff `desk_bookings` view (BK-6, #1388).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_bookings.ts")]
pub struct DeskBookingItemDto {
    pub id: String,
    pub booking_no: String,
    pub status: String,
    pub customer_display_name: Option<String>,
    pub phone_last4: Option<String>,
    pub readiness: String,
    pub ticket_number: Option<String>,
    pub slot_id: String,
    pub start_at: String,
    pub end_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_bookings.ts")]
pub struct DeskBookingsData {
    pub items: Vec<DeskBookingItemDto>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_bookings.ts")]
pub struct DeskBookingsFilter {
    pub qr: Option<String>,
    pub date: Option<String>,
}

impl ViewFilter for DeskBookingsFilter {}
