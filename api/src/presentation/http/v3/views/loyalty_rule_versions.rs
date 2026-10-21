//! LY-6 (#2868, for #2129): `GET /api/v3/views/loyalty_rule_versions` (design `membership-points.md` §6.4, §9.1):
//! lists all POINT and TIER rule versions held by Loyalty for the tenant, newest `version` first.

use std::path::Path;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_loyalty::application::rule_publisher::{RuleKind, RuleVersion};

use crate::bootstrap::AppState;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    authenticate_and_authorize, parse_branch, require_branch,
};

pub const PUBLIC_ROUTE: &str = "/api/v3/views/loyalty_rule_versions";

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "loyalty_rule_versions.ts")]
pub struct PointRuleVersionItem {
    pub version_id: String,
    pub version: i32,
    pub status: String,
    pub basis: String,
    pub points_ratio: String,
    pub rounding: String,
    pub minimum_basis_amount: String,
    pub expiry_months: Option<i32>,
    pub valid_from: String,
    pub valid_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "loyalty_rule_versions.ts")]
pub struct TierRuleVersionItem {
    pub version_id: String,
    pub version: i32,
    pub status: String,
    pub rolling_months: i32,
    #[ts(type = "unknown")]
    pub tiers: Value,
    #[ts(type = "unknown")]
    pub grace: Value,
    pub valid_from: String,
    pub valid_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "loyalty_rule_versions.ts")]
pub struct LoyaltyRuleVersionsData {
    pub point_versions: Vec<PointRuleVersionItem>,
    pub tier_versions: Vec<TierRuleVersionItem>,
}

fn loyalty_unavailable(message: impl Into<String>) -> V3Error {
    V3Error::new(
        "loyaltyUnavailable",
        None,
        message,
        StatusCode::SERVICE_UNAVAILABLE,
    )
}

fn req_str(obj: &serde_json::Map<String, Value>, key: &str) -> Result<String, V3Error> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| loyalty_unavailable(format!("missing or invalid {key}")))
}

fn opt_str(obj: &serde_json::Map<String, Value>, key: &str) -> Result<Option<String>, V3Error> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_owned()))
            .ok_or_else(|| loyalty_unavailable(format!("invalid {key}"))),
    }
}

fn req_i32(obj: &serde_json::Map<String, Value>, key: &str) -> Result<i32, V3Error> {
    obj.get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| loyalty_unavailable(format!("missing or invalid {key}")))
}

fn opt_i32(obj: &serde_json::Map<String, Value>, key: &str) -> Result<Option<i32>, V3Error> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| loyalty_unavailable(format!("invalid {key}"))),
    }
}

fn to_point_item(rv: RuleVersion) -> Result<PointRuleVersionItem, V3Error> {
    let obj = rv
        .content
        .as_object()
        .ok_or_else(|| loyalty_unavailable("point rule content is not an object"))?;
    Ok(PointRuleVersionItem {
        version_id: rv.id.to_string(),
        version: req_i32(obj, "version")?,
        status: rv.status,
        basis: req_str(obj, "basis")?,
        points_ratio: req_str(obj, "pointsRatio")?,
        rounding: req_str(obj, "rounding")?,
        minimum_basis_amount: req_str(obj, "minimumBasisAmount")?,
        expiry_months: opt_i32(obj, "expiryMonths")?,
        valid_from: req_str(obj, "validFrom")?,
        valid_to: opt_str(obj, "validTo")?,
    })
}

fn to_tier_item(rv: RuleVersion) -> Result<TierRuleVersionItem, V3Error> {
    let obj = rv
        .content
        .as_object()
        .ok_or_else(|| loyalty_unavailable("tier rule content is not an object"))?;
    let tiers = obj
        .get("tiers")
        .cloned()
        .ok_or_else(|| loyalty_unavailable("missing tiers"))?;
    let grace = obj
        .get("grace")
        .cloned()
        .ok_or_else(|| loyalty_unavailable("missing grace"))?;
    Ok(TierRuleVersionItem {
        version_id: rv.id.to_string(),
        version: req_i32(obj, "version")?,
        status: rv.status,
        rolling_months: req_i32(obj, "rollingMonths")?,
        tiers,
        grace,
        valid_from: req_str(obj, "validFrom")?,
        valid_to: opt_str(obj, "validTo")?,
    })
}

async fn handle(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let named =
        web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
            .ok()
            .and_then(|q| q.into_inner().remove("branch"));
    match parse_branch(named.as_deref()) {
        Ok(Some(branch)) => {
            if let Err(e) = require_branch(&req, &branch).await {
                return e.error_response();
            }
        }
        Ok(None) => {}
        Err(()) => {
            return V3Error::new(
                "invalidRequest",
                Some("branch"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    }

    let _who =
        match authenticate_and_authorize(&req, (EntryPath::Rpc, "loyalty_rule_versions")).await {
            Ok(who) => who,
            Err(refusal) => return refusal,
        };

    let Some(writer) = state.loyalty_rule_writer.as_ref() else {
        return loyalty_unavailable("loyalty service token is not configured").error_response();
    };

    let raw_points = match writer.list(RuleKind::Point).await {
        Ok(list) => list,
        Err(e) => return loyalty_unavailable(e.to_string()).error_response(),
    };
    let raw_tiers = match writer.list(RuleKind::Tier).await {
        Ok(list) => list,
        Err(e) => return loyalty_unavailable(e.to_string()).error_response(),
    };

    let mut point_versions = match raw_points
        .into_iter()
        .map(to_point_item)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(items) => items,
        Err(e) => return e.error_response(),
    };
    let mut tier_versions = match raw_tiers
        .into_iter()
        .map(to_tier_item)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(items) => items,
        Err(e) => return e.error_response(),
    };

    point_versions.sort_by_key(|a| std::cmp::Reverse(a.version));
    tier_versions.sort_by_key(|a| std::cmp::Reverse(a.version));

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(LoyaltyRuleVersionsData {
            point_versions,
            tier_versions,
        })
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    cfg.route(PUBLIC_ROUTE, web::get().to(handle));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PointRuleVersionItem::export_all(&cfg)?;
    TierRuleVersionItem::export_all(&cfg)?;
    LoyaltyRuleVersionsData::export_all(&cfg)?;
    Ok(())
}
