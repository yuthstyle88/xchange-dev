use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::value_objects::kiosk_event_kind::KioskEventKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub request: KioskRequest,
    pub event: Option<KioskEventKind>,
}
