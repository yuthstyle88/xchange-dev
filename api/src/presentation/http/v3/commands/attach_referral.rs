//! RA-3 (#981, design `broker-seam.md` §4.3): `POST /api/transactions/{id}/referral` (`attachReferral`).
//! Attach a customer's referral code to an exchange (O-2: TELLER of its branch, or EXECUTIVE).
//!
//! The desk captures with `source = DESK` and no `customer_sub` (the desk does not know the
//! Identity subject); `captured_by` is the caller. An exchange of another branch than the
//! authorised one is `outOfScope`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_referral::application::capture_service::{
    CaptureError, CaptureInput, ReferralCaptureService,
};
use xc_referral::domain::capture::{CaptureSource, CaptureStatus, CaptureTarget};
use xc_referral::domain::referral_code::ReferralCode;
use xc_referral::infrastructure::capture_repository::PgCaptureRepository;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::referral_attributions::{AttachReferralInput, AttachReferralOutput};
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

pub struct AttachReferralCommand {
    captures: ReferralCaptureService,
}

impl CommandAction for AttachReferralCommand {
    const NAME: &'static str = "attachReferral";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "attachReferral");
    type Input = AttachReferralInput;
    type Output = AttachReferralOutput;
    const ROUTE: &'static str = "/api/transactions/{id}/referral";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let transaction = Uuid::parse_str(&input.id).map_err(|_| no_such_target())?;
        let owner: Option<String> =
            sqlx::query_scalar("SELECT branch_id FROM transactions WHERE id = $1")
                .bind(transaction)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "attachReferral: read the exchange's branch");
                    server_error()
                })?;
        match owner {
            None => return Err(no_such_target()),
            Some(b) if b != branch.as_str() => return Err(out_of_scope()),
            Some(_) => {}
        }
        let code = ReferralCode::parse(&input.referral_code).map_err(capture_refusal)?;
        let id = self
            .captures
            .capture_in(
                tx,
                CaptureInput {
                    code,
                    on: CaptureTarget::Transaction(transaction),
                    source: CaptureSource::Desk,
                    customer_sub: None,
                    by: who.id().to_owned(),
                    at: chrono::Utc::now(),
                },
            )
            .await
            .map_err(capture_refusal)?;
        Ok(AttachReferralOutput {
            capture_id: id.to_string(),
            status: CaptureStatus::Active.as_str().to_owned(),
        })
    }
}

/// The capture service over PostgreSQL, auditing into `audit_events`.
pub(crate) fn capture_service(state: &AppState) -> ReferralCaptureService {
    ReferralCaptureService::new(
        Arc::new(PgCaptureRepository::new()),
        Arc::new(PgAuditLog::new(state.db.clone())),
    )
}

/// The answer to each `CaptureError` (`capture_service.rs`), shared by both referral commands.
pub(crate) fn capture_refusal(e: CaptureError) -> V3Error {
    match e {
        CaptureError::AlreadyCaptured => V3Error::new(
            "referralAlreadyCaptured",
            None,
            "รายการนี้มีรหัสแนะนำแล้ว ให้ใช้การเปลี่ยนรหัสแทน",
            StatusCode::CONFLICT,
        ),
        CaptureError::NotActive => V3Error::new(
            "staleVersion",
            None,
            "รหัสแนะนำนี้ถูกเปลี่ยนไปแล้ว",
            StatusCode::CONFLICT,
        ),
        CaptureError::NoSuchTarget => no_such_target(),
        CaptureError::NoCustomer | CaptureError::BadCode => V3Error::new(
            "validationFailed",
            Some("referralCode"),
            "รหัสแนะนำไม่ถูกต้อง",
            StatusCode::BAD_REQUEST,
        ),
        CaptureError::EmptyReason => V3Error::new(
            "validationFailed",
            Some("reason"),
            "ต้องระบุเหตุผล",
            StatusCode::BAD_REQUEST,
        ),
        CaptureError::Store(e) => {
            tracing::error!(error = %e, "referral capture store failed");
            server_error()
        }
    }
}

fn no_such_target() -> V3Error {
    V3Error::new(
        "invalidRequest",
        Some("id"),
        "คำขอไม่ถูกต้อง",
        StatusCode::BAD_REQUEST,
    )
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(AttachReferralCommand {
            captures: capture_service(state),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AttachReferralInput::export_all(&cfg)?;
    AttachReferralOutput::export_all(&cfg)?;
    Ok(())
}
