//! DTOs for the `admin_bookings` and `admin_booking_settings` views (§7, §12.1, §12.14, §12.17).

use std::path::Path;

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// Row in the `admin_bookings` page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_bookings.ts")]
pub struct AdminBookingRow {
    pub id: String,
    pub branch_id: String,
    pub name: String,
    pub phone: String,
    pub member: bool,
    #[serde(rename = "type")]
    pub r#type: String,
    pub currency: String,
    pub amount: String,
    pub rate: String,
    pub status: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub transaction_ref: Option<String>,
    pub counter_rate: Option<String>,
    pub counter_principal: Option<String>,
}

/// Filter for the `admin_bookings` page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_bookings.ts")]
pub struct AdminBookingFilter {
    #[ts(optional)]
    pub q: Option<String>,
}

impl ViewFilter for AdminBookingFilter {
    fn check(&self) -> Result<(), V3Error> {
        if let Some(ref q) = self.q
            && q.chars().count() > 100
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("q"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

/// Rules for booking settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_bookings.ts")]
pub struct AdminBookingRules {
    pub enabled: bool,
    #[ts(type = "number")]
    pub minutes: u32,
}

/// A counter rate item in booking settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_bookings.ts")]
pub struct AdminBookingCounterRate {
    pub currency: String,
    pub buy: String,
    pub sell: String,
}

/// Data for `admin_booking_settings` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_bookings.ts")]
pub struct AdminBookingSettingsData {
    pub rules: AdminBookingRules,
    #[ts(type = "number")]
    pub member_count: u64,
    pub public_url: String,
    pub counter_rates: Vec<AdminBookingCounterRate>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminBookingRow::export_all(&cfg)?;
    AdminBookingFilter::export_all(&cfg)?;
    AdminBookingRules::export_all(&cfg)?;
    AdminBookingCounterRate::export_all(&cfg)?;
    AdminBookingSettingsData::export_all(&cfg)?;
    Ok(())
}
