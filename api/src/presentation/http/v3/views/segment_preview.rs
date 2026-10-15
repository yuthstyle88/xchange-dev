//! MK-6 (#1404, design `marketing-campaigns.md` §4.5): `GET /api/v3/views/segment_preview`.
//! Preview of a snapshot: count, metric distribution, masked sample (masked names only, never identity documents).
//!
//! A named `?branch=` is checked first, for every caller, as `campaigns` and `my_profile` do: no v2 session maps to
//! a role holding `MarketingMembers`, so without it no caller could be answered `noSuchBranch` (#500,
//! `v3_view_unknown_branch_test::every_generated_view_refuses_an_unknown_branch`).

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::Utc;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::Masked;

use crate::bootstrap::AppState;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    BranchView, ViewAnswer, ViewFilter, authenticate_caller, parse_branch, require_branch,
};

pub use crate::presentation::dto::v3::segment_preview::{
    MaskedMemberSample, MemberMetricsDto, MetricDistribution, SegmentPreviewDto,
    SegmentPreviewFilter,
};

pub struct SegmentPreviewView {
    pool: PgPool,
}

impl SegmentPreviewView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        snapshot_id: Uuid,
    ) -> AppResult<SegmentPreviewDto> {
        let target = Target::new(&who.organization_id, Some(branch.clone()), None);
        Authorizer::check(Some(who), Permission::MarketingMembers, &target, Utc::now())
            .map_err(|r| AppError::Validation(r.code().to_string()))?;

        let org_id = Uuid::parse_str(&who.organization_id)
            .map_err(|_| AppError::Validation("roleNotAllowed".to_string()))?;

        let snap_row: Option<(i32,)> = sqlx::query_as(
            "SELECT member_count FROM segment_snapshots WHERE id = $1 AND organization_id = $2",
        )
        .bind(snapshot_id)
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        let Some((stored_member_count,)) = snap_row else {
            return Err(AppError::Validation("notFound".to_string()));
        };

        #[derive(sqlx::FromRow)]
        struct MemberRow {
            customer_id: String,
            display_name: String,
            metrics: serde_json::Value,
        }

        let rows: Vec<MemberRow> = sqlx::query_as(
            "SELECT m.customer_id, c.display_name, m.metrics \
             FROM segment_snapshot_members m \
             JOIN customers c ON c.id = m.customer_id \
             WHERE m.snapshot_id = $1 \
             ORDER BY m.customer_id",
        )
        .bind(snapshot_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        let member_count = stored_member_count as usize;
        let mut sample = Vec::with_capacity(rows.len().min(50));
        let mut total_freq: u64 = 0;
        let mut total_val = Decimal::ZERO;
        let mut recencies = Vec::new();

        for row in &rows {
            let metrics_dto: MemberMetricsDto = serde_json::from_value(row.metrics.clone())
                .unwrap_or(MemberMetricsDto {
                    recency_days: None,
                    frequency: 0,
                    value_thb: "0.00".to_string(),
                    tier_code: None,
                });

            if let Some(r) = metrics_dto.recency_days {
                recencies.push(r);
            }
            total_freq += metrics_dto.frequency as u64;

            if let Ok(v) = Decimal::from_str(&metrics_dto.value_thb) {
                total_val += v;
            }

            if sample.len() < 50 {
                sample.push(MaskedMemberSample {
                    customer_id: row.customer_id.clone(),
                    masked_name: Masked::of(&row.display_name).to_string(),
                    metrics: metrics_dto,
                });
            }
        }

        let avg_recency_days = if recencies.is_empty() {
            None
        } else {
            Some(recencies.iter().map(|&r| r as f64).sum::<f64>() / recencies.len() as f64)
        };

        let avg_frequency = if rows.is_empty() {
            0.0
        } else {
            total_freq as f64 / rows.len() as f64
        };

        let distribution = MetricDistribution {
            avg_recency_days,
            avg_frequency,
            total_value_thb: format!("{:.2}", total_val),
        };

        Ok(SegmentPreviewDto {
            snapshot_id: snapshot_id.to_string(),
            member_count,
            distribution,
            sample,
        })
    }
}

impl BranchView for SegmentPreviewView {
    const NAME: &'static str = "segment_preview";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "segment_preview");
    type Data = SegmentPreviewDto;
    type Filter = SegmentPreviewFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let who = who.clone();
        let filter = filter.clone();
        async move {
            let Some(id_str) = filter.id.as_deref() else {
                return Err(AppError::Validation("idRequired".to_string()));
            };
            let snap_id = Uuid::parse_str(id_str)
                .map_err(|_| AppError::Validation("invalidRequest".to_string()))?;
            let view = SegmentPreviewView::new(pool);
            view.load(&branch, &who, snap_id).await
        }
    }

    fn refusal(&self, err: &AppError) -> Option<V3Error> {
        match err {
            AppError::Validation(v) if v == "roleNotAllowed" => Some(V3Error::new(
                "roleNotAllowed",
                None,
                "role not allowed",
                StatusCode::BAD_REQUEST,
            )),
            AppError::Validation(v) if v == "notFound" => Some(V3Error::new(
                "notFound",
                Some("id"),
                "not found",
                StatusCode::NOT_FOUND,
            )),
            AppError::Validation(v) if v == "invalidRequest" || v == "idRequired" => Some(V3Error::new(
                "invalidRequest",
                Some("id"),
                "invalid request",
                StatusCode::BAD_REQUEST,
            )),
            _ => None,
        }
    }
}

async fn handle_segment_preview(req: &HttpRequest, view: &SegmentPreviewView) -> HttpResponse {
    let query = web::Query::<Vec<(String, String)>>::from_query(req.query_string())
        .map(|q| q.into_inner())
        .unwrap_or_default();
    let first = |key: &str| {
        query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let named = match parse_branch(first("branch").as_deref()) {
        Ok(named) => named,
        Err(()) => {
            return V3Error::new(
                "invalidRequest",
                Some("branch"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };
    if let Some(b) = &named
        && let Err(e) = require_branch(req, b).await
    {
        return e.error_response();
    }
    let caller = match authenticate_caller(req, SegmentPreviewView::INHERITS).await {
        Ok(caller) => caller,
        Err(refusal) => return refusal,
    };
    let now = chrono::Utc::now();
    let grant = match Policy::authorize(
        caller.user.as_ref(),
        SegmentPreviewView::INHERITS.0,
        &Action::new(SegmentPreviewView::INHERITS.1),
        named.as_ref(),
        now,
    ) {
        Ok(grant) => grant,
        Err(e) => return V3Error::from(e).error_response(),
    };
    let branch = grant.branch;
    if let Err(e) = require_branch(req, &branch).await {
        return e.error_response();
    }
    let filter = match web::Query::<SegmentPreviewFilter>::from_query(req.query_string()) {
        Ok(f) => f.into_inner(),
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
    if let Err(e) = filter.check() {
        return e.error_response();
    }
    let Some(state) = req.app_data::<web::Data<AppState>>() else {
        return V3Error::new(
            "serverError",
            None,
            "เซิร์ฟเวอร์ขัดข้อง",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .error_response();
    };
    let version = match BranchVersions::new(state.db.clone()).current(&branch).await {
        Ok(v) => v,
        Err(_) => {
            return V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };
    let since = first("since").and_then(|s| s.parse::<i64>().ok());
    if since == Some(version) {
        return HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(ViewAnswer::<SegmentPreviewDto> {
                version,
                changed: false,
                data: None,
            });
    }
    match view.load_as(&branch, &caller, &filter).await {
        Ok(data) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(ViewAnswer {
                version,
                changed: true,
                data: Some(data),
            }),
        Err(AppError::Unauthenticated) => V3Error::from(PolicyError::SignIn).error_response(),
        Err(err) => match view.refusal(&err) {
            Some(refusal) => refusal.error_response(),
            None => V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response(),
        },
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = Arc::new(SegmentPreviewView::new(state.db.clone()));
    cfg.route(
        &format!("/api/v3/views/{}", SegmentPreviewView::NAME),
        web::get().to(move |req: HttpRequest| {
            let view = view.clone();
            async move { handle_segment_preview(&req, view.as_ref()).await }
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::segment_preview::export_types(dir)
}

