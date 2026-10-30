//! #5630 W-1 — the audit `actor_role` the exchange workflows write (`SaveActor::audit_role`; design
//! `docs/design/org-wide-actor-branch.md` §14.5; rulings Q-1 A and Q-2 B on #5661: no grant reaches the save
//! step, so it is the saver's standing in Blueprint names, the owner and an executive `EXECUTIVE`).
//! Written before the implementation by the sub-controller (claude/studio-claude-17). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Today it is `StandingKind::audit_text`, V.9's spelling (`FRONT`, `MANAGER`, `RATES`, `SYSTEM`,
//! `DOCUMENT_REVIEW`, `OWNER`).

use xc_auth::domain::access::{Role, StandingKind};
use xchange::application::workflows::save_step::SaveActor;

fn saver(standing: StandingKind) -> SaveActor {
    SaveActor {
        id: "u1".to_owned(),
        username: "u1".to_owned(),
        employee_id: None,
        standing,
    }
}

/// S-1: every standing a saver can have, in Blueprint names.
#[test]
fn s1_every_standing_in_blueprint_names() {
    for (standing, text) in [
        (StandingKind::Owner, "EXECUTIVE"),
        (StandingKind::Executive, "EXECUTIVE"),
        (StandingKind::Staff(Role::Teller), "TELLER"),
        (StandingKind::Staff(Role::BranchManager), "BRANCH_MANAGER"),
        (StandingKind::Staff(Role::RateManager), "RATE_MANAGER"),
        (StandingKind::Staff(Role::Finance), "FINANCE"),
        (StandingKind::Staff(Role::Admin), "ADMIN"),
        (
            StandingKind::Staff(Role::VerificationReviewer),
            "VERIFICATION_REVIEWER",
        ),
        (StandingKind::Staff(Role::Marketing), "MARKETING"),
    ] {
        assert_eq!(saver(standing).audit_role(), text, "{standing:?}");
    }
}
