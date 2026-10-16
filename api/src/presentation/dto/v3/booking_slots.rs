//! DTOs for the public `booking_slots` view (BK-6, #1388).

use actix_web::http::StatusCode;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::error::V3Error;
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

impl ViewFilter for BookingSlotsFilter {
    fn check(&self) -> Result<(), V3Error> {
        if let Some(ref d) = self.date
            && NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err()
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("date"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}
