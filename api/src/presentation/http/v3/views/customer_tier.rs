//! LY-5 (#1408): `GET /api/v3/views/customer_tier?customer=` (design `membership-points.md` §4.6, M03):
//! one customer's mirrored tier.

use std::path::Path;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::customer_tier::CustomerTierData;
use crate::presentation::http::v3::command::out_of_scope;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    ViewAnswer, authenticate_and_authorize, parse_branch, require_branch,
};

pub const PUBLIC_ROUTE: &str = "/api/v3/views/customer_tier";

async fn handle(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    // A named branch first, for every caller, as the core checks it: an unknown one is `noSuchBranch` (#500). A
    // customer names none.
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

    let who = match authenticate_and_authorize(&req, (EntryPath::Rpc, "customer_tier")).await {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };

    let query = match web::Query::<std::collections::HashMap<String, String>>::from_query(
        req.query_string(),
    ) {
        Ok(q) => q.into_inner(),
        Err(_) => {
            return V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };

    let Some(customer_id) = query.get("customer").filter(|s| !s.trim().is_empty()) else {
        return V3Error::new(
            "invalidRequest",
            Some("customer"),
            "ต้องระบุลูกค้า",
            StatusCode::BAD_REQUEST,
        )
        .error_response();
    };

    let org_id = match Uuid::parse_str(&who.organization_id) {
        Ok(id) => id,
        Err(_) => return out_of_scope().error_response(),
    };

    let exists: bool = match sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM customers WHERE id = $1 AND organization_id = $2)",
    )
    .bind(customer_id)
    .bind(org_id)
    .fetch_one(&state.db)
    .await
    {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "database error checking customer existence");
            return V3Error::new(
                "serverError",
                None,
                "เกิดข้อผิดพลาดภายในระบบ",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    if !exists {
        return V3Error::new(
            "customerNotFound",
            Some("customer"),
            "ไม่พบข้อมูลลูกค้า",
            StatusCode::NOT_FOUND,
        )
        .error_response();
    }

    let tier_code: Option<String> = match sqlx::query_scalar(
        "SELECT m.tier_code \
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
    .bind(customer_id)
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

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(ViewAnswer {
            version: 0,
            changed: true,
            data: Some(CustomerTierData { tier: tier_code }),
        })
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    cfg.route(PUBLIC_ROUTE, web::get().to(handle));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CustomerTierData::export_all(&cfg)?;
    Ok(())
}
