//! LY-5 (#1408): `GET /api/v3/views/my_membership` (design `membership-points.md` §4.6, C05):
//! a customer's own tier and live points.

use std::path::Path;
use std::time::Duration;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::{DateTime, Utc};
use pos_infra::transactions::Transactor;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::customer::SystemReadPurpose;
use xc_customer::application::customer::service::CustomerService;
use xc_kernel::branch_tx::BranchTx;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::my_membership::{
    MyMembershipData, NextExpiryDto, PointsDto, TierProgressDto,
};
use crate::presentation::http::v3::command::out_of_scope;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    authenticate_and_authorize, parse_branch, require_branch,
};

pub const PUBLIC_ROUTE: &str = "/api/v3/views/my_membership";

async fn handle(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    // A named branch first, for every caller, as the core checks it: an unknown one is `noSuchBranch` (#500). A
    // customer names none.
    let named = web::Query::<std::collections::HashMap<String, String>>::from_query(
        req.query_string(),
    )
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

    let who = match authenticate_and_authorize(&req, (EntryPath::Rpc, "my_membership")).await {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };

    let Some(own_customer_id) = who.customer_id.as_deref() else {
        return out_of_scope().error_response();
    };

    if let Ok(query) = web::Query::<std::collections::HashMap<String, String>>::from_query(
        req.query_string(),
    ) && let Some(target) = query.get("customer")
        && target != own_customer_id
    {
        return out_of_scope().error_response();
    }

    let org_id = match Uuid::parse_str(&who.organization_id) {
        Ok(id) => id,
        Err(_) => return out_of_scope().error_response(),
    };

    // Owner check: customer row must belong to caller's session organisation
    let customer_org: Option<Uuid> = match sqlx::query_scalar(
        "SELECT organization_id FROM customers WHERE id = $1",
    )
    .bind(own_customer_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!(error = %e, "database error querying customer organisation");
            return V3Error::new(
                "serverError",
                None,
                "เกิดข้อผิดพลาดภายในระบบ",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    match customer_org {
        Some(co) if co == org_id => {}
        _ => return out_of_scope().error_response(),
    }

    // System read of customer's active verified phone
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let customer_service = CustomerService::new(branch_tx);
    let phone_opt = match customer_service
        .verified_phone_for_system(org_id, own_customer_id, SystemReadPurpose::Loyalty, Utc::now())
        .await
    {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!(error = %e, "error obtaining verified phone for system");
            return V3Error::new(
                "serverError",
                None,
                "เกิดข้อผิดพลาดภายในระบบ",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    let Some(phone) = phone_opt else {
        return HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(MyMembershipData {
                tier: None,
                effective_until: None,
                points: None,
                points_unavailable: false,
                next_expiry: None,
                progress: None,
            });
    };

    // Mirrored tier lookup
    let mirror_row: Option<(String, Option<DateTime<Utc>>)> = match sqlx::query_as(
        "SELECT m.tier_code, m.effective_until \
         FROM customer_identities i \
         JOIN customer_tier_mirror m \
           ON m.organization_id = i.organization_id \
          AND m.phone_lookup_hash = i.lookup_hash \
         WHERE i.organization_id = $1 \
           AND i.customer_id = $2 \
           AND i.type = 'PHONE' \
           AND i.status = 'ACTIVE' \
           AND i.verified_at IS NOT NULL \
         ORDER BY i.id DESC \
         LIMIT 1",
    )
    .bind(org_id)
    .bind(own_customer_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!(error = %e, "database error querying customer tier mirror");
            return V3Error::new(
                "serverError",
                None,
                "เกิดข้อผิดพลาดภายในระบบ",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    let (tier, effective_until) = match mirror_row {
        Some((t, u)) => (Some(t), u),
        None => (None, None),
    };

    // Live summary call with 2s timeout
    let summary_result = tokio::time::timeout(
        Duration::from_secs(2),
        state.loyalty_summary.summary(&phone),
    )
    .await;

    let (points, points_unavailable, next_expiry, progress) = match summary_result {
        Ok(Ok(summary)) => {
            let points = Some(PointsDto {
                available: summary.available,
                pending: summary.pending,
            });
            let next_expiry = summary.next_expiry.map(|ne| NextExpiryDto {
                points: ne.points,
                at: ne.at,
            });
            let progress = summary.tier.and_then(|t| t.progress).map(|p| TierProgressDto {
                basis: p.basis,
                current: p.current,
                next: p.next,
            });
            (points, false, next_expiry, progress)
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "loyalty summary returned error");
            (None, true, None, None)
        }
        Err(_) => {
            tracing::warn!("loyalty summary timed out after 2s");
            (None, true, None, None)
        }
    };

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(MyMembershipData {
            tier,
            effective_until,
            points,
            points_unavailable,
            next_expiry,
            progress,
        })
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    cfg.route(PUBLIC_ROUTE, web::get().to(handle));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PointsDto::export_all(&cfg)?;
    NextExpiryDto::export_all(&cfg)?;
    TierProgressDto::export_all(&cfg)?;
    MyMembershipData::export_all(&cfg)?;
    Ok(())
}
