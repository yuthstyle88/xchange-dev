//! Admin transactions view DTOs for v3 (§7, §12.1, §12.2).

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::application::executive::channel::Channel;
use crate::presentation::dto::v3::common::Period;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// Filter keys accepted when drilling down from executive overview (G2, ruling on #1104).
pub const DRILLDOWN_KEYS: &[&str] = &["from", "to", "currency", "channel"];

/// Transaction type (side) for filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "admin_transactions.ts")]
pub enum AdminTransactionType {
    BUY,
    SELL,
}

/// Transaction status for filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "admin_transactions.ts")]
pub enum AdminTransactionStatus {
    COMPLETED,
    CANCELLED,
    PENDING,
}

/// Filter for the `admin_transactions` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_transactions.ts")]
pub struct AdminTransactionsFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub q: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "AdminTransactionType")]
    pub r#type: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "AdminTransactionStatus")]
    pub status: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub staff: Option<String>,

    #[serde(default, rename = "staffQ", skip_serializing_if = "Option::is_none")]
    #[ts(rename = "staffQ", optional)]
    pub staff_q: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub currency: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub channel: Option<String>,
}

impl ViewFilter for AdminTransactionsFilter {
    fn check(&self) -> Result<(), V3Error> {
        let period = Period {
            from: self.from.clone().unwrap_or_default(),
            to: self.to.clone().unwrap_or_default(),
        };
        period.check()?;

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

        if let Some(ref t) = self.r#type
            && !t.is_empty()
            && t != "BUY"
            && t != "SELL"
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("type"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }

        if let Some(ref s) = self.status
            && !s.is_empty()
            && s != "COMPLETED"
            && s != "CANCELLED"
            && s != "PENDING"
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("status"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }

        if let Some(ref c) = self.currency
            && (c.len() != 3 || !c.chars().all(|ch| ch.is_ascii_uppercase()))
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("currency"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }

        if let Some(ref ch) = self.channel
            && Channel::parse(ch).is_none()
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("channel"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }

        Ok(())
    }
}
