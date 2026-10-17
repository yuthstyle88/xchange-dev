//! MK-7 (#1406): `GET /api/v3/views/campaigns` (M01, design `marketing-campaigns.md` §4.5, D-2): every campaign of
//! the caller's organisation with its status (derived for an APPROVED version), budget used/left, uses and the
//! period totals of the branch the core resolved. Gate: `MarketingCampaigns` (policy row `campaigns`).
//!
//! A named `?branch=` is checked first, for every caller, as `my_profile` does: no v2 session maps to a role holding
//! `MarketingCampaigns`, so without it no caller could be answered `noSuchBranch` (#500,
//! `v3_view_unknown_branch_test::every_generated_view_refuses_an_unknown_branch`). [`serve_branch_first`] is that
//! order on the seam's envelope; `campaign` uses it too.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::campaigns::{
    CampaignListItem, CampaignPeriodTotals, CampaignsData, CampaignsFilter,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    BranchView, ViewAnswer, ViewFilter, authenticate_caller, parse_branch, require_branch,
};

impl ViewFilter for CampaignsFilter {
    fn check(&self) -> Result<(), V3Error> {
        let Ok(from) = chrono::NaiveDate::parse_from_str(&self.from, "%Y-%m-%d") else {
            return Err(V3Error::new(
                "invalidRequest",
                Some("from"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        };
        let Ok(to) = chrono::NaiveDate::parse_from_str(&self.to, "%Y-%m-%d") else {
            return Err(V3Error::new(
                "invalidRequest",
                Some("to"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        };
        if from > to {
            return Err(V3Error::new(
                "periodOrder",
                Some("to"),
                "วันที่เริ่มต้นต้องไม่เกินวันที่สิ้นสุด",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

pub struct CampaignsView {
    pool: PgPool,
}

impl CampaignsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for CampaignsView {
    const NAME: &'static str = "campaigns";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "campaigns");
    type Data = CampaignsData;
    type Filter = CampaignsFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let org_id = who.organization_id.clone();
        let filter = filter.clone();

        async move {
            let from = chrono::NaiveDate::parse_from_str(&filter.from, "%Y-%m-%d")
                .map_err(|_| pos_error::AppError::Validation("invalid from date".into()))?;
            let to = chrono::NaiveDate::parse_from_str(&filter.to, "%Y-%m-%d")
                .map_err(|_| pos_error::AppError::Validation("invalid to date".into()))?;

            let rows = sqlx::query(
                r#"
                SELECT
                    c.id AS campaign_id,
                    c.name AS campaign_name,
                    cv.id AS version_id,
                    cv.version AS version,
                    cv.status AS status,
                    cv.starts_at AS starts_at,
                    cv.ends_at AS ends_at,
                    cv.budget_thb AS budget_thb,
                    cv.max_uses AS max_uses,
                    COALESCE(cb.consumed_thb, 0)::numeric(20,2) AS consumed_thb,
                    COALESCE(cb.used_count, 0)::bigint AS used_count,
                    COALESCE((
                        SELECT SUM(r.amount_thb)
                        FROM benefit_reservations r
                        WHERE r.campaign_version_id = cv.id
                          AND r.status = 'RESERVED'
                          AND r.expires_at > now()
                    ), 0)::numeric(20,2) AS actively_reserved_thb,
                    COALESCE(pt.redemptions, 0)::bigint AS redemptions,
                    COALESCE(pt.price_cost, 0)::numeric(20,2) AS price_cost,
                    COALESCE(pt.reward_expense, 0)::numeric(20,2) AS reward_expense
                FROM campaigns c
                JOIN LATERAL (
                    SELECT *
                    FROM campaign_versions
                    WHERE id = COALESCE(
                        c.current_version_id,
                        (SELECT id FROM campaign_versions WHERE campaign_id = c.id ORDER BY version DESC LIMIT 1)
                    )
                ) cv ON true
                LEFT JOIN campaign_budgets cb ON cb.campaign_version_id = cv.id
                LEFT JOIN LATERAL (
                    SELECT
                        COUNT(br.id)::bigint AS redemptions,
                        COALESCE(SUM(CASE WHEN br.benefit_class = 'PRICE' THEN br.amount_thb ELSE 0 END), 0)::numeric(20,2) AS price_cost,
                        COALESCE(SUM(CASE WHEN br.benefit_class = 'REWARD' THEN br.amount_thb ELSE 0 END), 0)::numeric(20,2) AS reward_expense
                    FROM benefit_redemptions br
                    JOIN campaign_versions br_cv ON br_cv.id = br.campaign_version_id
                    JOIN transactions t ON t.id = br.transaction_id
                    JOIN branches b ON b.id = t.branch_id
                    WHERE br_cv.campaign_id = c.id
                      AND br.status = 'REDEEMED'
                      AND t.branch_id = $2
                      AND (br.redeemed_at AT TIME ZONE b.timezone)::date >= $3
                      AND (br.redeemed_at AT TIME ZONE b.timezone)::date <= $4
                ) pt ON true
                WHERE c.organization_id = $1::uuid
                ORDER BY c.created_at ASC, c.name ASC
                "#,
            )
            .bind(&org_id)
            .bind(branch.as_str())
            .bind(from)
            .bind(to)
            .fetch_all(&pool)
            .await?;

            use sqlx::Row;
            let now = chrono::Utc::now();
            let mut campaigns = Vec::with_capacity(rows.len());

            for r in rows {
                let campaign_id: uuid::Uuid = r.get("campaign_id");
                let campaign_name: String = r.get("campaign_name");
                let version_id: uuid::Uuid = r.get("version_id");
                let version: i32 = r.get("version");
                let stored_status: String = r.get("status");
                let starts_at: chrono::DateTime<chrono::Utc> = r.get("starts_at");
                let ends_at: chrono::DateTime<chrono::Utc> = r.get("ends_at");
                let mut budget_thb: rust_decimal::Decimal = r.get("budget_thb");
                budget_thb.rescale(2);
                let max_uses: Option<i32> = r.get("max_uses");
                let mut consumed_thb: rust_decimal::Decimal = r.get("consumed_thb");
                consumed_thb.rescale(2);
                let used_count: i64 = r.get("used_count");
                let mut actively_reserved_thb: rust_decimal::Decimal = r.get("actively_reserved_thb");
                actively_reserved_thb.rescale(2);
                let redemptions: i64 = r.get("redemptions");
                let mut price_cost: rust_decimal::Decimal = r.get("price_cost");
                price_cost.rescale(2);
                let mut reward_expense: rust_decimal::Decimal = r.get("reward_expense");
                reward_expense.rescale(2);

                let status = derive_version_status(&stored_status, starts_at, ends_at, now);
                let mut budget_left = budget_thb - consumed_thb - actively_reserved_thb;
                budget_left.rescale(2);

                campaigns.push(CampaignListItem {
                    id: campaign_id.to_string(),
                    name: campaign_name,
                    version_id: version_id.to_string(),
                    version: version as i64,
                    status,
                    starts_at: starts_at.to_rfc3339(),
                    ends_at: ends_at.to_rfc3339(),
                    budget_thb: crate::presentation::dto::v3::common::Money(format!("{budget_thb:.2}")),
                    budget_used_thb: crate::presentation::dto::v3::common::Money(format!("{consumed_thb:.2}")),
                    budget_left_thb: crate::presentation::dto::v3::common::Money(format!("{budget_left:.2}")),
                    uses: used_count,
                    max_uses: max_uses.map(|m| m as i64),
                    period: CampaignPeriodTotals {
                        redemptions,
                        price_cost_thb: crate::presentation::dto::v3::common::Money(format!("{price_cost:.2}")),
                        reward_expense_thb: crate::presentation::dto::v3::common::Money(format!("{reward_expense:.2}")),
                    },
                });
            }

            Ok(CampaignsData { campaigns })
        }
    }
}

pub(crate) fn derive_version_status(
    status: &str,
    starts_at: chrono::DateTime<chrono::Utc>,
    ends_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    if status == "APPROVED" {
        if now < starts_at {
            "SCHEDULED".to_string()
        } else if now >= ends_at {
            "EXPIRED".to_string()
        } else {
            "ACTIVE".to_string()
        }
    } else {
        status.to_string()
    }
}

fn invalid_request(field: Option<&'static str>) -> HttpResponse {
    V3Error::new(
        "invalidRequest",
        field,
        "คำขอไม่ถูกต้อง",
        StatusCode::BAD_REQUEST,
    )
    .error_response()
}

fn server_error() -> HttpResponse {
    V3Error::new(
        "serverError",
        None,
        "เซิร์ฟเวอร์ขัดข้อง",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .error_response()
}

/// The view core's order with the named branch first: `?branch=` ill-formed ⇒ `invalidRequest`, unknown ⇒
/// `noSuchBranch`; then the policy (`Policy::authorize` with the named branch, as the core); then the filter, the
/// branch's version and `since`, and `load_as` into the seam's envelope.
pub(crate) async fn serve_branch_first<V: BranchView>(req: &HttpRequest, view: &V) -> HttpResponse {
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
        Err(()) => return invalid_request(Some("branch")),
    };
    if let Some(b) = &named
        && let Err(e) = require_branch(req, b).await
    {
        return e.error_response();
    }
    let caller = match authenticate_caller(req, V::INHERITS).await {
        Ok(caller) => caller,
        Err(refusal) => return refusal,
    };
    let now = chrono::Utc::now();
    let grant = match Policy::authorize(
        caller.user.as_ref(),
        V::INHERITS.0,
        &Action::new(V::INHERITS.1),
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
    let filter = match web::Query::<V::Filter>::from_query(req.query_string()) {
        Ok(f) => f.into_inner(),
        Err(_) => return invalid_request(None),
    };
    if let Err(e) = filter.check() {
        return e.error_response();
    }
    let Some(state) = req.app_data::<web::Data<AppState>>() else {
        return server_error();
    };
    let version = match BranchVersions::new(state.db.clone()).current(&branch).await {
        Ok(v) => v,
        Err(_) => return server_error(),
    };
    let since = first("since").and_then(|s| s.parse::<i64>().ok());
    if V::VERSIONED && since == Some(version) {
        return HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(ViewAnswer::<V::Data> {
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
        Err(_) => server_error(),
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = Arc::new(CampaignsView::new(state.db.clone()));
    cfg.route(
        &format!("/api/v3/views/{}", CampaignsView::NAME),
        web::get().to(move |req: HttpRequest| {
            let view = view.clone();
            async move { serve_branch_first(&req, view.as_ref()).await }
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CampaignsFilter::export_all(&cfg)?;
    CampaignsData::export_all(&cfg)?;
    CampaignListItem::export_all(&cfg)?;
    CampaignPeriodTotals::export_all(&cfg)?;
    Ok(())
}
