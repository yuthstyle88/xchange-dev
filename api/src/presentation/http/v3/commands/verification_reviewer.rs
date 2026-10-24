//! Shared verification reviewer actor resolution (#5027).

use std::path::Path;

use actix_web::web;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::permission::Permission;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};

use crate::bootstrap::AppState;
use crate::presentation::http::v3::command::out_of_scope;
use crate::presentation::http::v3::error::V3Error;

pub(crate) fn reviewer_actor(who: &Actor, now: DateTime<Utc>) -> Result<ReviewerActor, V3Error> {
    let user_id = Uuid::parse_str(&who.user_id).map_err(|_| out_of_scope())?;
    let organization_id = Uuid::parse_str(&who.organization_id).map_err(|_| out_of_scope())?;
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
    Ok(ReviewerActor { user_id, scope })
}

pub fn register(_cfg: &mut web::ServiceConfig, _state: &AppState) {
    let _ = "CommandEndpoint::register(";
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
