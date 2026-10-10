//! Customer entity.

use chrono::{DateTime, Utc};
use xc_kernel::mode::Mode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Customer {
    pub id: String,
    pub name: String,
    pub nationality: String,
    pub mode: Mode,
    pub created_at: DateTime<Utc>,
    pub last_document_ref: Option<String>,
}
