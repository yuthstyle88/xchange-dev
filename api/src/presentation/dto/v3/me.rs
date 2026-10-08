//! Me DTO for v3 (§7.5).

use serde::Serialize;
use ts_rs::TS;

use super::common::Me;

/// View DTO for the `me` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export_to = "me.ts")]
pub struct MeView(pub Me);
