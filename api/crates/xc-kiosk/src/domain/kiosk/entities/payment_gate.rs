//! Whether the branch's payment rules let a kiosk quote or confirm through (design §5.5.2,
//! row PM-K). Pure: `KioskMachine` decides first, and the gate only adds v2's payment check in
//! v2's place (`server.mjs:7976-7978` and `:7988-7994` of the 10-06 bundle).

use xc_kernel::payment::PaymentRules;

use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::value_objects::kiosk_error::KioskError;
use crate::domain::kiosk::value_objects::kiosk_event_kind::KioskEventKind;
use crate::domain::kiosk::value_objects::transition::Transition;

/// The request's channel is off for its direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Payment method unavailable. Please contact staff.")]
pub struct PaymentUnavailable;

/// Why a confirm was refused: the machine's own sentence, or the channel being off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfirmRefusal {
    #[error(transparent)]
    Machine(KioskError),
    #[error(transparent)]
    Unavailable(PaymentUnavailable),
}

/// The payment check of the kiosk.
pub struct PaymentGate;

impl PaymentGate {
    /// A quote the machine made passes only when its method is on for its direction.
    pub fn quote(request: &KioskRequest, rules: &PaymentRules) -> Result<(), PaymentUnavailable> {
        if Self::allowed(request, rules) {
            Ok(())
        } else {
            Err(PaymentUnavailable)
        }
    }

    /// v2 checks the method after the 3-second check and before the rate: when the machine
    /// moved the request to WAITING, or refused it for a changed rate, an off method wins.
    /// Any other answer — an earlier refusal, or the identity of a request already past
    /// QUOTED — passes through unchanged.
    pub fn confirm(
        machine: Result<Transition, KioskError>,
        stored: Option<&KioskRequest>,
        rules: &PaymentRules,
    ) -> Result<Transition, ConfirmRefusal> {
        let payment_decides = match &machine {
            Ok(moved) => moved.event == Some(KioskEventKind::Request),
            Err(e) => *e == KioskError::RateChangedOrClosed,
        };
        if payment_decides && let Some(request) = stored {
            Self::quote(request, rules).map_err(ConfirmRefusal::Unavailable)?;
        }
        machine.map_err(ConfirmRefusal::Machine)
    }

    fn allowed(request: &KioskRequest, rules: &PaymentRules) -> bool {
        rules.allows(request.quote.payment_method.as_str(), request.quote.side)
    }
}
