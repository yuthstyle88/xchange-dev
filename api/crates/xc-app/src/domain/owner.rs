//! Exactly one owner per draft (design P6).

use crate::domain::app_session::SessionHash;

/// Who owns a planner draft: a customer, or an anonymous app session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DraftOwner {
    /// `planner_drafts.customer_id`.
    Customer(String),
    /// `planner_drafts.session_hash`.
    Session(SessionHash),
}
