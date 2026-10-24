//! v3 commands module (§6).

use actix_web::http::StatusCode;
use sqlx::PgPool;
use std::sync::Arc;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_kernel::money::{CurrencyCode, Rate};
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;

use crate::bootstrap::approval_subjects::{ApprovalSubjectDeps, registry_with};
use crate::presentation::http::v3::error::V3Error;

pub fn make_approval_registry(pool: PgPool) -> ApprovalRegistry {
    crate::bootstrap::approval_subjects::registry(pool)
}

pub fn make_approval_service_with(deps: &ApprovalSubjectDeps) -> Arc<ApprovalService> {
    let registry = registry_with(deps);
    let repo = Arc::new(PgApprovalRepository::new(deps.pool.clone()));
    let audit_log = Arc::new(PgAuditLog::new(deps.pool.clone()));
    Arc::new(ApprovalService::new(registry, repo, audit_log))
}

pub fn make_approval_service(pool: PgPool) -> Arc<ApprovalService> {
    make_approval_service_with(&ApprovalSubjectDeps::pool_only(pool))
}

pub fn map_rate_engine_error(err: RateEngineError) -> V3Error {
    match err {
        RateEngineError::NotFound(m) => V3Error::new("notFound", None, m, StatusCode::NOT_FOUND),
        RateEngineError::ValidationFailed(m) => {
            V3Error::new("validationFailed", None, m, StatusCode::BAD_REQUEST)
        }
        RateEngineError::OutOfScope(m) => {
            V3Error::new("outOfScope", None, m, StatusCode::FORBIDDEN)
        }
        RateEngineError::StaleVersion(m) => {
            V3Error::new("staleVersion", None, m, StatusCode::CONFLICT)
        }
        RateEngineError::RateUnavailable(m) => {
            V3Error::new("rateUnavailable", None, m, StatusCode::CONFLICT)
        }
        RateEngineError::ApprovalRequired(m) => {
            V3Error::new("approvalRequired", None, m, StatusCode::CONFLICT)
        }
        RateEngineError::IdempotencyConflict(m) => {
            V3Error::new("idempotencyConflict", None, m, StatusCode::CONFLICT)
        }
        RateEngineError::Refused { code, message } => {
            V3Error::new(code, None, message, StatusCode::CONFLICT)
        }
        _ => V3Error::new(
            "serverError",
            None,
            "เซิร์ฟเวอร์ขัดข้อง",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    }
}

pub fn parse_draft_line(
    currency: &str,
    buy_mode: &str,
    sell_mode: &str,
    manual_buy: Option<&str>,
    manual_sell: Option<&str>,
    reason: Option<String>,
) -> Result<DraftLine, V3Error> {
    let curr = CurrencyCode::parse(currency).map_err(|e| {
        V3Error::new(
            "validationFailed",
            Some("currency"),
            e.to_string(),
            StatusCode::BAD_REQUEST,
        )
    })?;

    let buy = match buy_mode {
        "AUTO" => LineSide::Auto,
        "MANUAL" => {
            let s = manual_buy.ok_or_else(|| {
                V3Error::new(
                    "validationFailed",
                    Some("manualBuy"),
                    "missing manual buy",
                    StatusCode::BAD_REQUEST,
                )
            })?;
            let r = Rate::parse(s).map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("manualBuy"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;
            LineSide::Manual(r)
        }
        _ => {
            return Err(V3Error::new(
                "validationFailed",
                Some("buyMode"),
                "invalid buy mode",
                StatusCode::BAD_REQUEST,
            ));
        }
    };

    let sell = match sell_mode {
        "AUTO" => LineSide::Auto,
        "MANUAL" => {
            let s = manual_sell.ok_or_else(|| {
                V3Error::new(
                    "validationFailed",
                    Some("manualSell"),
                    "missing manual sell",
                    StatusCode::BAD_REQUEST,
                )
            })?;
            let r = Rate::parse(s).map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("manualSell"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;
            LineSide::Manual(r)
        }
        _ => {
            return Err(V3Error::new(
                "validationFailed",
                Some("sellMode"),
                "invalid sell mode",
                StatusCode::BAD_REQUEST,
            ));
        }
    };

    Ok(DraftLine::new(curr, buy, sell, reason))
}
