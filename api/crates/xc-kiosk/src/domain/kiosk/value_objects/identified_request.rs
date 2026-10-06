//! A request together with what the customer screen is told about it.

use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::value_objects::request_identity::RequestIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiedRequest {
    pub request: KioskRequest,
    pub identity: RequestIdentity,
}
