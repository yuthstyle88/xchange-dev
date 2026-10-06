use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Employee {
    pub id: String,
    pub username: String,
    pub name: String,
    pub role: String,
    pub branch_id: String,
    pub active: bool,
    pub must_change: bool,
    pub version: i32,
    pub created_at: DateTime<Utc>,
}
