use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub id: String,
    pub username: String,
    pub name: String,
    pub active: bool,
    pub version: i32,
    pub created_at: DateTime<Utc>,
}
