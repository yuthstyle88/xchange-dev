//! `GET /api/public/orgs/{org}/consent_texts?lang=th|en|zh` (#3409, design `section-b-gaps.md` §S6):
//! the newest published version of each consent purpose in `lang`, with an `ETag` and `304` when unchanged.

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;
use xc_customer::application::consent_service::ConsentService;

use crate::bootstrap::AppState;
use crate::presentation::http::public::not_found_response;
use crate::presentation::http::v3::error::V3Error;

#[derive(Deserialize, Default)]
struct ConsentTextsQuery {
    lang: Option<String>,
}

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone())).route(
        "/api/public/orgs/{org}/consent_texts",
        web::get().to(handle),
    );
}

async fn handle(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    query: Option<web::Query<ConsentTextsQuery>>,
) -> HttpResponse {
    let lang = query.and_then(|q| q.into_inner().lang).unwrap_or_default();
    if !matches!(lang.as_str(), "th" | "en" | "zh") {
        return V3Error::new(
            "validationFailed",
            Some("lang"),
            "invalid lang",
            StatusCode::BAD_REQUEST,
        )
        .error_response();
    }

    let org_code = path.into_inner();
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("failed to begin transaction for public consent_texts: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let org_id: Option<Uuid> =
        match sqlx::query_scalar("SELECT id FROM organizations WHERE code = $1")
            .bind(&org_code)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("failed to resolve organization for public consent_texts: {e}");
                return HttpResponse::InternalServerError().finish();
            }
        };

    let Some(org_id) = org_id else {
        return not_found_response();
    };

    // Over every published text of the language, not only the answered ones: versions are numbered per purpose,
    // so the answer's own count and max can stay put when one purpose publishes (TERMS v1 + MARKETING v2, then
    // TERMS v2). `consent_texts` is append-only, so the count rises on every publish.
    let (count, max_version): (i64, i32) = match sqlx::query_as(
        "SELECT count(*), COALESCE(MAX(version), 0) FROM consent_texts \
         WHERE organization_id = $1 AND lang = $2",
    )
    .bind(org_id)
    .bind(&lang)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(tag) => tag,
        Err(e) => {
            tracing::error!("failed to compute etag for public consent_texts: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let etag = format!("\"c{count}-v{max_version}\"");

    if let Some(if_none_match) = req
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && (if_none_match == etag || if_none_match.trim() == etag.trim())
    {
        return HttpResponse::NotModified()
            .insert_header((header::ETAG, etag))
            .finish();
    }

    let rows = match ConsentService::current_texts_in(&mut tx, &org_id.to_string(), &lang).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("failed to fetch public consent_texts: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    HttpResponse::Ok()
        .insert_header((header::ETAG, etag))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(rows)
}
