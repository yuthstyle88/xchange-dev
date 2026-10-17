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

    /// An idempotency-key replay answers the first record only to the user who wrote it (#2848).
    /// The key is unique per branch, not per user, so another user's key must not disclose that record.
    pub fn replay_is_own(record_counted_by: &str, actor: &Actor) -> Result<(), CashError> {
        if record_counted_by != actor.user_id {
            Err(CashError::ReplayOfAnotherUser)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(user_id: &str) -> Actor {
        Actor::new(user_id, "org-1", Vec::new(), None)
    }

    #[test]
    fn replay_by_the_writer_is_allowed() {
        assert_eq!(
            PersonRules::replay_is_own("teller-a", &actor("teller-a")),
            Ok(())
        );
    }

    #[test]
    fn replay_by_another_user_is_refused() {
        let err = PersonRules::replay_is_own("teller-a", &actor("teller-b")).unwrap_err();
        assert_eq!(err, CashError::ReplayOfAnotherUser);
        assert_eq!(err.code(), "replayOfAnotherUser");
        assert_eq!(err.to_string(), "รหัสคำขอนี้ถูกใช้โดยผู้ใช้อื่นแล้ว");
    }
}
