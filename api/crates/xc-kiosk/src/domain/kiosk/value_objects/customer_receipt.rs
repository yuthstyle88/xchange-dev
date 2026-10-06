//! The receipt as the customer screen asks for it.

use crate::domain::kiosk::value_objects::request_identity::RequestIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerReceipt {
    pub identity: RequestIdentity,
    /// The published picture while it may still be read; `None` before and after.
    pub image: Option<String>,
}
