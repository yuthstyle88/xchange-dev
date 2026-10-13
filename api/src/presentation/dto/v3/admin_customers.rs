//! Admin customers view DTOs for v3 (§7, §12.1).

use std::path::Path;

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::Period;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// Filter keys accepted when drilling down from executive overview (G2, ruling on #1104).
pub const DRILLDOWN_KEYS: &[&str] = &["q", "from", "to", "repeat"];

/// Row in the `admin_customers` page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_customers.ts")]
pub struct AdminCustomerRow {
    pub id: String,
    pub name: String,
    pub nationality: String,
    pub created: String,
    pub member: bool,
    #[ts(type = "number")]
    pub completed: i64,
    pub volume: String,
    pub last_at: Option<String>,
}

/// Filter for the `admin_customers` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_customers.ts")]
pub struct AdminCustomersFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub q: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub repeat: Option<bool>,
}

impl ViewFilter for AdminCustomersFilter {
    fn check(&self) -> Result<(), V3Error> {
        let period = Period {
            from: self.from.clone().unwrap_or_default(),
            to: self.to.clone().unwrap_or_default(),
        };
        period.check()?;

        if self.repeat == Some(true) && self.from.as_deref().unwrap_or("").is_empty() {
            return Err(V3Error::new(
                "invalidRequest",
                Some("from"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }

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

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminCustomerRow::export_all(&cfg)?;
    AdminCustomersFilter::export_all(&cfg)?;
    Ok(())
}
