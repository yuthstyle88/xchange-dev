//! MK-6 (#1404, design `marketing-campaigns.md` §4.5): `POST /api/segments` (`segmentDefine`).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_marketing::domain::segment::rule::SegmentRule;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize)]
pub struct SegmentDefineInput {
    pub name: String,
    pub rule: SegmentRule,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentDefineOutput {
    pub id: Uuid,
    pub version: i32,
}

pub struct SegmentDefineCommand;

impl CommandAction for SegmentDefineCommand {
    const NAME: &'static str = "segmentDefine";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "segmentDefine");
    type Input = SegmentDefineInput;
    type Output = SegmentDefineOutput;
    const ROUTE: &'static str = "/api/segments";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let target = Target::new(&who.organization_id, Some(branch.clone()), None);
        Authorizer::check(Some(who), Permission::MarketingMembers, &target, Utc::now())
            .map_err(|_| V3Error::from(PolicyError::RoleNotAllowed))?;

        let org_id = Uuid::parse_str(&who.organization_id)
            .map_err(|_| V3Error::from(PolicyError::RoleNotAllowed))?;
        let user_id = Uuid::parse_str(&who.user_id).unwrap_or_else(|_| Uuid::nil());

        if input.name.trim().is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("name"),
                "name cannot be empty",
                StatusCode::BAD_REQUEST,
            ));
        }

        let next_version: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0) + 1 \
             FROM segment_definitions \
             WHERE organization_id = $1 AND name = $2",
        )
        .bind(org_id)
        .bind(&input.name)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

        let id = Uuid::new_v4();
        let rule_json = serde_json::to_value(&input.rule).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("rule"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;

        sqlx::query(
            "INSERT INTO segment_definitions (id, organization_id, name, version, rule, created_by, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now())",
        )
        .bind(id)
        .bind(org_id)
        .bind(&input.name)
        .bind(next_version)
        .bind(rule_json)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

        Ok(SegmentDefineOutput {
            id,
            version: next_version,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(SegmentDefineCommand));
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
