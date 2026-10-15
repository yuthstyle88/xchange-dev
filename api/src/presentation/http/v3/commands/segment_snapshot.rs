//! MK-6 (#1404, design `marketing-campaigns.md` §4.5): `POST /api/segments/{id}/snapshots` (`segmentSnapshot`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_marketing::application::segment_service::{SegmentError, SegmentSnapshotService};
use xc_marketing::domain::engine::consent_gate::ConsentGate;
use xc_marketing::domain::segment::rule::SegmentRule;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::marketing::pg_customer_metrics::PgCustomerMetrics;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize)]
pub struct SegmentSnapshotInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentSnapshotOutput {
    pub id: Uuid,
    #[serde(rename = "memberCount")]
    pub member_count: usize,
}

pub struct SegmentSnapshotCommand;

struct PrefetchedConsentGate {
    allowed: HashMap<String, bool>,
}

impl ConsentGate for PrefetchedConsentGate {
    fn marketing_allowed(&self, customer: &str) -> bool {
        self.allowed.get(customer).copied().unwrap_or(false)
    }
}

fn map_segment_error(e: SegmentError) -> V3Error {
    match e {
        SegmentError::NotFound(_) => V3Error::new(
            "notFound",
            Some("id"),
            "segment definition not found",
            StatusCode::NOT_FOUND,
        ),
        SegmentError::ValidationFailed(msg) => {
            V3Error::new("validationFailed", None, msg, StatusCode::BAD_REQUEST)
        }
        SegmentError::OutOfScope => {
            V3Error::new("outOfScope", None, "out of scope", StatusCode::BAD_REQUEST)
        }
        SegmentError::Database(err) => {
            V3Error::new("database", None, err.to_string(), StatusCode::INTERNAL_SERVER_ERROR)
        }
        SegmentError::IdempotencyConflict => V3Error::new(
            "idempotencyConflict",
            None,
            "idempotency conflict",
            StatusCode::CONFLICT,
        ),
    }
}

impl CommandAction for SegmentSnapshotCommand {
    const NAME: &'static str = "segmentSnapshot";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "segmentSnapshot");
    type Input = SegmentSnapshotInput;
    type Output = SegmentSnapshotOutput;
    const ROUTE: &'static str = "/api/segments/{id}/snapshots";
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

        let def_id = Uuid::parse_str(&input.id).map_err(|_| {
            V3Error::new(
                "invalidRequest",
                Some("id"),
                "id must be uuid",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let as_of = Utc::now();

        let rule = SegmentSnapshotService::rule_of_in(tx, org_id, def_id)
            .await
            .map_err(map_segment_error)?;

        let lookback = rule
            .lookback_months
            .unwrap_or_else(SegmentRule::default_lookback_months);

        let candidates = PgCustomerMetrics::load_candidate_metrics_in(tx, org_id, lookback, as_of)
            .await
            .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

        let consent_svc = xc_customer::application::consent_service::ConsentService::pg(
            PgPool::connect_lazy("postgres://127.0.0.1/none").expect("lazy pool"),
        );

        let mut allowed_map = HashMap::new();
        for candidate in &candidates {
            if !rule.matches(candidate) {
                continue;
            }
            let allowed = consent_svc
                .marketing_allowed_in(tx, &candidate.customer_id, xc_customer::domain::consent::Channel::Sms)
                .await
                .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?
                || consent_svc
                    .marketing_allowed_in(tx, &candidate.customer_id, xc_customer::domain::consent::Channel::Email)
                    .await
                    .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?
                || consent_svc
                    .marketing_allowed_in(tx, &candidate.customer_id, xc_customer::domain::consent::Channel::Push)
                    .await
                    .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;
            allowed_map.insert(candidate.customer_id.clone(), allowed);
        }

        let consent_gate = PrefetchedConsentGate { allowed: allowed_map };

        let (snap_id, member_count) = SegmentSnapshotService::snapshot_in(
            tx,
            org_id,
            def_id,
            user_id,
            as_of,
            &candidates,
            &consent_gate,
        )
        .await
        .map_err(map_segment_error)?;

        Ok(SegmentSnapshotOutput {
            id: snap_id,
            member_count,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(SegmentSnapshotCommand));
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
