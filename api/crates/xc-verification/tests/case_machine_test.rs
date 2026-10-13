//! Pure unit tests for `CaseMachine` reviewer-side transitions (design §4.2, §4.3).

use chrono::{Duration, Utc};
use uuid::Uuid;
use xc_verification::domain::case::case_error::CaseError;
use xc_verification::domain::case::case_machine::CaseMachine;
use xc_verification::domain::case::case_status::CaseStatus;
use xc_verification::domain::case::decision_reason::DecisionReason;
use xc_verification::domain::case::provider::Provider;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::domain::case::verification_case::VerificationCase;

fn make_case(status: CaseStatus) -> VerificationCase {
    VerificationCase {
        id: Uuid::new_v4(),
        organization_id: Uuid::new_v4(),
        customer_id: "cust-test-1".to_string(),
        provider: Provider::Staff,
        status,
        policy_version: 1,
        branch_id: None,
        submitted_at: None,
        decided_at: None,
        expires_at: None,
        reviewed_by: None,
        decision_reason: None,
        version: 1,
    }
}

fn reviewer() -> ReviewerActor {
    ReviewerActor {
        user_id: Uuid::new_v4(),
        scope: ReviewerScope::Organization {
            organization_id: Uuid::new_v4(),
        },
    }
}

#[test]
fn start_review_allowed_only_from_submitted() {
    let rev = reviewer();
    let now = Utc::now();

    for &s in &CaseStatus::ALL {
        let case = make_case(s);
        let res = CaseMachine::start_review(&case, &rev, now);
        if s == CaseStatus::Submitted {
            let updated = res.expect("start_review from SUBMITTED must succeed");
            assert_eq!(updated.status, CaseStatus::UnderReview);
            assert_eq!(updated.version, case.version + 1);
        } else {
            assert!(
                matches!(res, Err(CaseError::CaseLocked)),
                "start_review from {s:?} must return CaseLocked"
            );
        }
    }
}

#[test]
fn approve_allowed_only_from_under_review() {
    let rev = reviewer();
    let now = Utc::now();
    let exp = now + Duration::days(365);

    for &s in &CaseStatus::ALL {
        let case = make_case(s);
        let res = CaseMachine::approve(&case, &rev, exp, now);
        if s == CaseStatus::UnderReview {
            let updated = res.expect("approve from UNDER_REVIEW must succeed");
            assert_eq!(updated.status, CaseStatus::Verified);
            assert_eq!(updated.reviewed_by, Some(rev.user_id));
            assert_eq!(updated.decided_at, Some(now));
            assert_eq!(updated.expires_at, Some(exp));
            assert_eq!(updated.decision_reason, None);
            assert_eq!(updated.version, case.version + 1);
        } else {
            assert!(
                matches!(res, Err(CaseError::CaseLocked)),
                "approve from {s:?} must return CaseLocked"
            );
        }
    }
}

#[test]
fn approve_from_submitted_is_refused() {
    let case = make_case(CaseStatus::Submitted);
    let rev = reviewer();
    let now = Utc::now();
    let exp = now + Duration::days(365);

    let res = CaseMachine::approve(&case, &rev, exp, now);
    assert!(
        matches!(res, Err(CaseError::CaseLocked)),
        "approve from SUBMITTED must be refused"
    );
}

#[test]
fn reject_allowed_only_from_under_review() {
    let rev = reviewer();
    let now = Utc::now();
    let reason = DecisionReason::DocumentExpired;

    for &s in &CaseStatus::ALL {
        let case = make_case(s);
        let res = CaseMachine::reject(&case, &rev, reason, now);
        if s == CaseStatus::UnderReview {
            let updated = res.expect("reject from UNDER_REVIEW must succeed");
            assert_eq!(updated.status, CaseStatus::Rejected);
            assert_eq!(updated.reviewed_by, Some(rev.user_id));
            assert_eq!(updated.decided_at, Some(now));
            assert_eq!(updated.decision_reason, Some(reason));
            assert_eq!(updated.version, case.version + 1);
        } else {
            assert!(
                matches!(res, Err(CaseError::CaseLocked)),
                "reject from {s:?} must return CaseLocked"
            );
        }
    }
}

#[test]
fn request_update_allowed_only_from_under_review() {
    let rev = reviewer();
    let now = Utc::now();
    let reason = DecisionReason::DocumentUnreadable;

    for &s in &CaseStatus::ALL {
        let case = make_case(s);
        let res = CaseMachine::request_update(&case, &rev, reason, now);
        if s == CaseStatus::UnderReview {
            let updated = res.expect("request_update from UNDER_REVIEW must succeed");
            assert_eq!(updated.status, CaseStatus::NeedsUpdate);
            assert_eq!(updated.reviewed_by, Some(rev.user_id));
            assert_eq!(updated.decided_at, Some(now));
            assert_eq!(updated.decision_reason, Some(reason));
            assert_eq!(updated.version, case.version + 1);
        } else {
            assert!(
                matches!(res, Err(CaseError::CaseLocked)),
                "request_update from {s:?} must return CaseLocked"
            );
        }
    }
}

#[test]
fn revoke_allowed_only_from_verified() {
    let rev = reviewer();
    let now = Utc::now();
    let reason = DecisionReason::SuspectedFraud;

    for &s in &CaseStatus::ALL {
        let case = make_case(s);
        let res = CaseMachine::revoke(&case, &rev, reason, now);
        if s == CaseStatus::Verified {
            let updated = res.expect("revoke from VERIFIED must succeed");
            assert_eq!(updated.status, CaseStatus::Revoked);
            assert_eq!(updated.reviewed_by, Some(rev.user_id));
            assert_eq!(updated.decided_at, Some(now));
            assert_eq!(updated.decision_reason, Some(reason));
            assert_eq!(updated.version, case.version + 1);
        } else {
            assert!(
                matches!(res, Err(CaseError::CaseLocked)),
                "revoke from {s:?} must return CaseLocked"
            );
        }
    }
}

#[test]
fn revoke_from_non_verified_states_is_refused() {
    let rev = reviewer();
    let now = Utc::now();
    let reason = DecisionReason::SuspectedFraud;

    let non_verified = [
        CaseStatus::Draft,
        CaseStatus::Submitted,
        CaseStatus::UnderReview,
        CaseStatus::Rejected,
        CaseStatus::NeedsUpdate,
        CaseStatus::Expired,
        CaseStatus::Revoked,
    ];

    for s in non_verified {
        let case = make_case(s);
        let res = CaseMachine::revoke(&case, &rev, reason, now);
        assert!(
            matches!(res, Err(CaseError::CaseLocked)),
            "revoke from non-VERIFIED state {s:?} must be refused"
        );
    }
}

#[test]
fn expire_allowed_only_from_verified_past_expires_at() {
    let now = Utc::now();
    let past = now - Duration::seconds(10);

    for &s in &CaseStatus::ALL {
        let mut case = make_case(s);
        case.expires_at = Some(past);
        let res = CaseMachine::expire(&case, now);
        if s == CaseStatus::Verified {
            let updated = res.expect("expire on VERIFIED past expires_at must succeed");
            assert_eq!(updated.status, CaseStatus::Expired);
            assert_eq!(updated.version, case.version + 1);
        } else {
            assert!(
                matches!(res, Err(CaseError::CaseLocked)),
                "expire from {s:?} must return CaseLocked"
            );
        }
    }
}

#[test]
fn expire_before_expiry_date_is_refused() {
    let now = Utc::now();
    let future = now + Duration::days(30);

    let mut case = make_case(CaseStatus::Verified);
    case.expires_at = Some(future);

    let res = CaseMachine::expire(&case, now);
    assert!(
        matches!(res, Err(CaseError::CaseLocked)),
        "expire before expires_at must be refused"
    );
}

#[test]
fn expire_with_no_expires_at_is_refused() {
    let now = Utc::now();

    let mut case = make_case(CaseStatus::Verified);
    case.expires_at = None;

    let res = CaseMachine::expire(&case, now);
    assert!(
        matches!(res, Err(CaseError::CaseLocked)),
        "expire with no expires_at must be refused"
    );
}
