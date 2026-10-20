//! C05 (#3433): `GET /api/v3/views/my_benefits` (design `marketing-campaigns.md` §6.3, O-3):
//! a customer's own usable personal coupons (never the code) and open APP benefits.

use std::path::Path;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::Utc;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::money::Side;
use xc_kernel::pii::LookupHasher;
use xc_loyalty::domain::membership_mirror::MembershipMirror;
use xc_loyalty::infrastructure::pg_membership_mirror::PgMembershipMirror;
use xc_marketing::domain::benefit::listing::{
    BenefitSummary, CustomerFacts, line_conditions, shown_on_app, summary,
};
use xc_marketing::domain::benefit::registry::CalculatorRegistry;
use xc_marketing::domain::benefit::rule::BenefitRule;
use xc_marketing::domain::engine::consent_gate::ConsentGate;
use xc_marketing::domain::engine::referral_source::ReferralSource;
use xc_marketing::infrastructure::pg_customer_benefits::{BranchRef, PgCustomerBenefits};

use crate::bootstrap::AppState;
use crate::infrastructure::marketing::consent_gate::MarketingConsentGate;
use crate::infrastructure::marketing::referral_source::MarketingReferralSource;
use crate::presentation::dto::v3::my_benefits::{
    BenefitConditionsDto, BenefitSummaryDto, BranchRefDto, MyBenefitDto, MyBenefitsData,
    MyCouponDto,
};
use crate::presentation::http::v3::command::out_of_scope;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    authenticate_and_authorize, parse_branch, require_branch,
};

pub const PUBLIC_ROUTE: &str = "/api/v3/views/my_benefits";

fn server_error() -> HttpResponse {
    V3Error::new(
        "serverError",
        None,
        "เกิดข้อผิดพลาดภายในระบบ",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .error_response()
}

fn to_summary_dto(s: BenefitSummary) -> BenefitSummaryDto {
    BenefitSummaryDto {
        benefit_type: s.benefit_type,
        benefit_class: s.benefit_class.as_str().to_string(),
        delta: s.delta.map(|d| d.to_string()),
        percent: s.percent.map(|d| d.to_string()),
        amount: s.amount.map(|d| d.to_string()),
        fixed: s.fixed.map(|d| d.to_string()),
    }
}

fn to_conditions_dto(rule: &BenefitRule, branches: Vec<BranchRef>) -> BenefitConditionsDto {
    let lc = line_conditions(&rule.eligibility);
    BenefitConditionsDto {
        min_thb: lc.min_thb.map(|d| d.to_string()),
        currencies: lc.currencies,
        shop_side: lc.shop_side.map(|side| match side {
            Side::Buy => "BUY".to_string(),
            Side::Sell => "SELL".to_string(),
        }),
        branches: branches
            .into_iter()
            .map(|b| BranchRefDto {
                branch_code: b.branch_code,
                name: b.name,
            })
            .collect(),
    }
}

async fn handle(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
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

    let who = match authenticate_and_authorize(&req, (EntryPath::Rpc, "my_benefits")).await {
        Ok(who) => who,
        Err(mut refusal) => {
            if refusal.status() == StatusCode::BAD_REQUEST {
                *refusal.status_mut() = StatusCode::FORBIDDEN;
            }
            return refusal;
        }
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
            return server_error();
        }
    };

    match customer_org {
        Some(co) if co == org_id => {}
        _ => return out_of_scope().error_response(),
    }

    let keys = match xc_customer::get_pii_keys() {
        Ok(k) => k,
        Err(e) => {
            tracing::error!(error = %e, "missing PII keys for my_benefits");
            return server_error();
        }
    };

    let tier = match PgMembershipMirror::new(state.db.clone(), LookupHasher::new(&keys))
        .tier_of(org_id, own_customer_id)
        .await
    {
        Ok(opt) => opt.map(|m| m.tier_code),
        Err(e) => {
            tracing::error!(error = %e, "error reading customer tier mirror");
            return server_error();
        }
    };

    let now = Utc::now();
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "failed to begin transaction for my_benefits");
            return server_error();
        }
    };

    let segment_memberships: Vec<Uuid> = match sqlx::query_scalar(
        "SELECT m.snapshot_id \
         FROM segment_snapshot_members m \
         JOIN segment_snapshots s ON s.id = m.snapshot_id \
         WHERE s.organization_id = $1 AND m.customer_id = $2",
    )
    .bind(org_id)
    .bind(own_customer_id)
    .fetch_all(&mut *tx)
    .await
    {
        Ok(ids) => ids,
        Err(e) => {
            tracing::error!(error = %e, "error querying segment memberships");
            return server_error();
        }
    };

    let consent_gate = match MarketingConsentGate::prefetch_in(&mut tx, own_customer_id).await {
        Ok(gate) => gate,
        Err(e) => {
            tracing::error!(error = %e, "error prefetching marketing consent");
            return server_error();
        }
    };

    let referral_source =
        match MarketingReferralSource::prefetch_in(&mut tx, own_customer_id, now).await {
            Ok(src) => src,
            Err(e) => {
                tracing::error!(error = %e, "error prefetching referral attribution");
                return server_error();
            }
        };

    let raw_coupons =
        match PgCustomerBenefits::personal_coupons_in(&mut tx, org_id, own_customer_id, now).await {
            Ok(list) => list,
            Err(e) => {
                tracing::error!(error = %e, "error reading personal coupons");
                return server_error();
            }
        };

    let raw_rules = match PgCustomerBenefits::open_rules_in(&mut tx, org_id, now).await {
        Ok(list) => list,
        Err(e) => {
            tracing::error!(error = %e, "error reading open benefit rules");
            return server_error();
        }
    };

    let facts = CustomerFacts {
        customer_id: own_customer_id,
        tier: tier.as_deref(),
        segment_memberships: &segment_memberships,
        marketing_consent: consent_gate.marketing_allowed(own_customer_id),
        is_referred: referral_source.referred(own_customer_id, now),
    };

    let registry = CalculatorRegistry::new();

    let mut coupons = Vec::with_capacity(raw_coupons.len());
    for cp in raw_coupons {
        match summary(&cp.rule, &registry) {
            Ok(s) => {
                let conditions = to_conditions_dto(&cp.rule, cp.branches);
                coupons.push(MyCouponDto {
                    coupon_id: cp.coupon_id,
                    label: cp.label,
                    summary: to_summary_dto(s),
                    conditions,
                    expires_at: cp.expires_at,
                });
            }
            Err(e) => {
                tracing::warn!(
                    rule_id = %cp.rule.id,
                    benefit_type = %cp.rule.benefit_type,
                    field = e.field,
                    reason = %e.reason,
                    "skipping personal coupon rule with invalid summary params"
                );
            }
        }
    }

    let mut benefits = Vec::new();
    for open in raw_rules {
        if !shown_on_app(&open.rule, open.segment_snapshot_id, &facts) {
            continue;
        }
        match summary(&open.rule, &registry) {
            Ok(s) => {
                let conditions = to_conditions_dto(&open.rule, open.branches);
                benefits.push(MyBenefitDto {
                    rule_id: open.rule.id,
                    label: open.label,
                    summary: to_summary_dto(s),
                    conditions,
                    ends_at: open.ends_at,
                });
            }
            Err(e) => {
                tracing::warn!(
                    rule_id = %open.rule.id,
                    benefit_type = %open.rule.benefit_type,
                    field = e.field,
                    reason = %e.reason,
                    "skipping open benefit rule with invalid summary params"
                );
            }
        }
    }

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(MyBenefitsData { coupons, benefits })
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    cfg.route(PUBLIC_ROUTE, web::get().to(handle));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchRefDto::export_all(&cfg)?;
    BenefitSummaryDto::export_all(&cfg)?;
    BenefitConditionsDto::export_all(&cfg)?;
    MyCouponDto::export_all(&cfg)?;
    MyBenefitDto::export_all(&cfg)?;
    MyBenefitsData::export_all(&cfg)?;
    Ok(())
}
