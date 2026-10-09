//! Organization sub-context.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub code: String,
    pub legal_name: Option<String>,
    pub timezone: String,
}
