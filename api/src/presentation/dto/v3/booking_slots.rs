//! DTOs for the public `booking_slots` view (BK-6, #1388).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "booking_slots.ts")]
pub struct BookingSlotItemDto {
    pub id: String,
    pub start: String,
    pub end: String,
    pub available: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "booking_slots.ts")]
pub struct BookingSlotsData {
    pub as_of: String,
    pub slots: Vec<BookingSlotItemDto>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "booking_slots.ts")]
pub struct BookingSlotsFilter {
    pub date: Option<String>,
}

impl ViewFilter for BookingSlotsFilter {}
