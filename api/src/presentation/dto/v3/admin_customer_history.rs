//! Admin customer history view DTOs for v3 (§7, §12.1).

use std::path::Path;

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::transaction_row::TransactionRow;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// Filter for the `admin_customer_history` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_customer_history.ts")]
pub struct AdminCustomerHistoryFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub customer: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub q: Option<String>,
}

impl ViewFilter for AdminCustomerHistoryFilter {
    fn check(&self) -> Result<(), V3Error> {
        match &self.customer {
            Some(c) if !c.trim().is_empty() => {}
            _ => {
                return Err(V3Error::new(
                    "invalidRequest",
                    Some("customer"),
                    "คำขอไม่ถูกต้อง",
                    StatusCode::BAD_REQUEST,
                ));
            }
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
    AdminCustomerHistoryFilter::export_all(&cfg)?;
    TransactionRow::export_all(&cfg)?;
    Ok(())
}
