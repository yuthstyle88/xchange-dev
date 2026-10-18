//! Custody actor session info.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyActor {
    pub id: String,
    pub employee_id: Option<String>,
}
