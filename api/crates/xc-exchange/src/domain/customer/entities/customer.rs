//! Customer entity.

use chrono::{DateTime, NaiveDate, Utc};
use xc_kernel::ids::BranchId;

use crate::domain::customer::value_objects::mode::Mode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Customer {
    pub id: String,
    pub branch_id: BranchId,
    pub name: String,
    pub passport_no: String,
    pub nationality: String,
    pub birth_date: Option<NaiveDate>,
    pub passport_expiry: Option<NaiveDate>,
    pub mode: Mode,
    pub created_at: DateTime<Utc>,
    pub last_document_ref: Option<String>,
}
