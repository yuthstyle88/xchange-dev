//! What a command answers when its posting refuses `stockInsufficient` (design `ledger-inventory.md` §5.1):
//! the `NEGATIVE_STOCK` request is opened (or re-read) and the caller is told to wait for it. One place for
//! the exchange completion (#3405), the reversal post and the transfer dispatch (XC-BE-GAPS-A rows 2, 3).

use actix_web::http::StatusCode;
use pos_error::AppError;
use sqlx::PgPool;
use xc_approval::application::approval_service::ApprovalService;
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;

use crate::application::workflows::exchange::negative_stock_gate::{
    NegativeStockGate, NegativeStockObject, NegativeStockOutcome,
};
use crate::presentation::http::v3::command::server_error;
use crate::presentation::http::v3::error::V3Error;

/// The refusal's message when `e` is the posting port's `stockInsufficient`.
pub fn stock_insufficient(e: &AppError) -> Option<&str> {
    match e {
        AppError::Domain(m) if m.starts_with("stockInsufficient") => Some(m.as_str()),
        _ => None,
    }
}

/// Opens or re-reads the `NEGATIVE_STOCK` request for `object` and answers `409 approvalRequired: <id>`,
/// or `409 stockInsufficient` when the request was rejected.
pub async fn stock_refusal_answer(
    pool: &PgPool,
    approvals: &ApprovalService,
    object: &NegativeStockObject,
    branch: &BranchId,
    refusal: &str,
    who: &Actor,
    key: &str,
) -> V3Error {
    let outcome = match NegativeStockGate::open_after_refusal(
        pool, approvals, object, branch, refusal, who, key,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(_) => return server_error(),
    };
    match outcome {
        NegativeStockOutcome::ApprovalRequired(req_id) => V3Error::new(
            "approvalRequired",
            None,
            format!("approvalRequired: {req_id}"),
            StatusCode::CONFLICT,
        ),
        NegativeStockOutcome::Rejected(req_id) => V3Error::new(
            "stockInsufficient",
            None,
            format!("stockInsufficient: negative stock approval request {req_id} was rejected"),
            StatusCode::CONFLICT,
        ),
    }
}
