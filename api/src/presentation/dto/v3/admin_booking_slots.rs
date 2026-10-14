//! DTOs for the manager/admin `admin_booking_slots` view (BK-6, #1388).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_booking_slots.ts")]
pub struct AdminSlotRuleDto {
    pub branch_id: String,
    pub slot_minutes: i32,
    pub capacity: i32,
    pub horizon_days: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_booking_slots.ts")]
pub struct AdminClosureDto {
    pub branch_id: String,
    pub closed_on: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_booking_slots.ts")]
pub struct AdminSlotDto {
    pub id: String,
    pub branch_id: String,
    pub service_date: String,
    pub start_at: String,
    pub end_at: String,
    pub capacity: i32,
    pub reserved: i32,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_booking_slots.ts")]
pub struct AdminBookingPolicyDto {
    pub cancel_cutoff_minutes: i32,
    pub no_show_grace_minutes: i32,
    pub confirm_expiry_minutes: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_booking_slots.ts")]
pub struct AdminBookingSlotsData {
    pub rule: Option<AdminSlotRuleDto>,
    pub closures: Vec<AdminClosureDto>,
    pub slots: Vec<AdminSlotDto>,
    pub policy: Option<AdminBookingPolicyDto>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_booking_slots.ts")]
pub struct AdminBookingSlotsFilter {
    pub date: Option<String>,
}

impl ViewFilter for AdminBookingSlotsFilter {}
