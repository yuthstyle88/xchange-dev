//! `couponIssue` (MK-4, design `marketing-campaigns.md` §4.5, §7).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "coupon_issue.ts", rename_all = "camelCase")]
pub struct CouponIssueInput {
    pub campaign_id: Option<String>,
    #[serde(default)]
    #[ts(skip)]
    pub id: Option<String>,
    pub rule_id: String,
    pub count: u32,
    pub customer_id: Option<String>,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "coupon_issue.ts", rename_all = "camelCase")]
pub struct IssuedCouponDto {
    pub id: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "coupon_issue.ts", rename_all = "camelCase")]
pub struct CouponIssueOutput {
    pub coupons: Vec<IssuedCouponDto>,
}

pub struct CouponIssueCommand;

impl CommandAction for CouponIssueCommand {
    const NAME: &'static str = "couponIssue";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "couponIssue");
    type Input = CouponIssueInput;
    type Output = CouponIssueOutput;
    const ROUTE: &'static str = "/api/campaigns/{id}/coupons";
    const METHOD: Method = Method::POST;

    fn stored_output(&self, output: &Self::Output) -> serde_json::Value {
        let mut val = serde_json::to_value(output).unwrap_or_default();
        if let Some(coupons) = val.get_mut("coupons").and_then(|c| c.as_array_mut()) {
            for coupon in coupons {
                if let Some(obj) = coupon.as_object_mut() {
                    obj.remove("code");
                }
            }
        }
        val
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let org_id = Uuid::parse_str(&who.organization_id).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("organizationId"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let rule_id = Uuid::parse_str(&input.rule_id).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("ruleId"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;

        let campaign_id = input
            .id
            .as_deref()
            .or(input.campaign_id.as_deref())
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| {
                V3Error::new(
                    "ruleNotFound",
                    Some("ruleId"),
                    "benefit rule not found",
                    StatusCode::NOT_FOUND,
                )
            })?;

        let rule_exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT br.id \
             FROM benefit_rules br \
             JOIN campaign_versions cv ON br.campaign_version_id = cv.id \
             WHERE br.id = $1 AND cv.organization_id = $2 AND cv.campaign_id = $3",
        )
        .bind(rule_id)
        .bind(org_id)
        .bind(campaign_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| {
            V3Error::new(
                "database",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        if rule_exists.is_none() {
            return Err(V3Error::new(
                "ruleNotFound",
                Some("ruleId"),
                "benefit rule not found",
                StatusCode::NOT_FOUND,
            ));
        }

        let expires_at = chrono::DateTime::parse_from_rfc3339(&input.expires_at)
            .map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("expiresAt"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?
            .with_timezone(&Utc);

        let user_id = resolve_user_id(tx, who, org_id).await;

        let keys = xc_customer::get_pii_keys().map_err(|_| server_error())?;
        let hasher = CouponCodeHasher::new(&keys);

        let mut issued_coupons = Vec::with_capacity(input.count as usize);

        for _ in 0..input.count {
            let coupon_id = Uuid::new_v4();
            let code = format!(
                "CPN-{}",
                Uuid::new_v4().simple().to_string().to_ascii_uppercase()
            );
            let code_hash = hasher.hash(&code);

            sqlx::query(
                "INSERT INTO coupons ( \
                     id, organization_id, rule_id, code_hash, customer_id, \
                     expires_at, status, issued_by, issued_at \
                 ) VALUES ($1, $2, $3, $4, $5, $6, 'ISSUED', $7, now())",
            )
            .bind(coupon_id)
            .bind(org_id)
            .bind(rule_id)
            .bind(&code_hash[..])
            .bind(input.customer_id.as_deref())
            .bind(expires_at)
            .bind(user_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| {
                V3Error::new(
                    "database",
                    None,
                    e.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            })?;

            issued_coupons.push(IssuedCouponDto {
                id: coupon_id.to_string(),
                code: Some(code),
            });
        }

        Ok(CouponIssueOutput {
            coupons: issued_coupons,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(CouponIssueCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    IssuedCouponDto::export_all(&cfg)?;
    CouponIssueInput::export_all(&cfg)?;
    CouponIssueOutput::export_all(&cfg)?;
    Ok(())
}

async fn resolve_user_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    who: &Actor,
    org_id: Uuid,
) -> Uuid {
    if let Ok(u) = Uuid::parse_str(&who.user_id) {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
            .bind(u)
            .fetch_one(&mut **tx)
            .await
            .unwrap_or(false);
        if exists {
            return u;
        }
    }
    let fallback: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE organization_id = $1 LIMIT 1")
            .bind(org_id)
            .fetch_optional(&mut **tx)
            .await
            .unwrap_or(None);
    fallback.unwrap_or_else(|| Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap())
}
