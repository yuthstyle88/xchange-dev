//! Shared verification reviewer actor resolution (#5027).

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::access::{Actor, Role};
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};

use crate::presentation::http::v3::command::out_of_scope;
use crate::presentation::http::v3::error::V3Error;

pub(crate) fn reviewer_actor(who: &Actor, now: DateTime<Utc>) -> Result<ReviewerActor, V3Error> {
    let user_id = Uuid::parse_str(&who.user_id).map_err(|_| out_of_scope())?;
    let organization_id = Uuid::parse_str(&who.organization_id).map_err(|_| out_of_scope())?;
    let role = who
        .roles
        .iter()
        .find(|r| {
            r.valid_from <= now
                && r.valid_to.is_none_or(|to| now < to)
                && RoleGrants::holds(r.group_code(), Permission::VerificationReview)
        })
        .map_or(Role::VerificationReviewer, |r| r.role);
    let scope = match BranchScope::of(who, Permission::VerificationReview, now) {
        BranchScope::All => ReviewerScope::Organization { organization_id },
        BranchScope::Branches(bs) => {
            let branch_id = match &who.branch {
                Some(b) if bs.contains(b) => b.clone(),
                Some(_) => return Err(out_of_scope()),
                None if bs.len() == 1 => bs[0].clone(),
                None => return Err(out_of_scope()),
            };
            ReviewerScope::Branch {
                organization_id,
                branch_id,
            }
        }
    };
    Ok(ReviewerActor {
        role,
        user_id,
        scope,
    })
}
