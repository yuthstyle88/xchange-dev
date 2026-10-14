//! DTOs for the customer `my_bookings` view (BK-6, #1388).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_bookings.ts")]
pub struct MyBookingItemDto {
    pub id: String,
    pub booking_no: String,
    pub status: String,
    pub branch_id: String,
    pub slot_id: String,
    pub start_at: String,
    pub end_at: String,
    pub qr_token: Option<String>,
    pub quote_summary: String,
    pub cancel_cutoff_at: Option<String>,
    pub ticket_number: Option<String>,
    pub readiness: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_bookings.ts")]
pub struct MyBookingsData {
    pub bookings: Vec<MyBookingItemDto>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_bookings.ts")]
pub struct MyBookingsFilter {
    pub booking_id: Option<String>,
}

impl ViewFilter for MyBookingsFilter {}
