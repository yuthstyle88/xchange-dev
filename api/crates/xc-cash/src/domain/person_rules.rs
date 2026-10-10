//! Person rules for cash domain.

use xc_auth::domain::access::Actor;

use super::error::CashError;

pub struct PersonRules;

impl PersonRules {
    pub fn verifier_is_not_counter(counter: &str, verifier: &Actor) -> Result<(), CashError> {
        if counter == verifier.user_id {
            Err(CashError::SameCounter)
        } else {
            Ok(())
        }
    }

    pub fn receiver_verifies_handover(receiver: &str, actor: &Actor) -> Result<(), CashError> {
        if receiver != actor.user_id {
            Err(CashError::NotReceiver)
        } else {
            Ok(())
        }
    }

    pub fn teller_owns_shift(shift_teller: &str, actor: &Actor) -> Result<(), CashError> {
        if shift_teller != actor.user_id {
            Err(CashError::NotOwnShift)
        } else {
            Ok(())
        }
    }
}
