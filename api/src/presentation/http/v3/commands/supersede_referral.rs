//! RA-3 (#981, design `broker-seam.md` §4.3): `POST /api/referral-attributions/{id}/supersede` (`supersedeReferral`).
//! Replace an ACTIVE referral code, with a reason (O-2: EXECUTIVE only).
//!
//! The capture service writes the new ACTIVE row, marks the old one `SUPERSEDED` and appends one
//! audit event naming the actor and the reason. A capture of another branch than the authorised
//! one is `outOfScope`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_referral::application::capture_service::{CaptureError, ReferralCaptureService};
use xc_referral::domain::capture::CaptureId;
use xc_referral::domain::referral_code::ReferralCode;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::referral_attributions::{SupersedeReferralInput, SupersedeReferralOutput};
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::attach_referral::{capture_refusal, capture_service};
use crate::presentation::http::v3::error::V3Error;

pub struct SupersedeReferralCommand {
    captures: ReferralCaptureService,
}

impl CommandAction for SupersedeReferralCommand {
    const NAME: &'static str = "supersedeReferral";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "supersedeReferral");
    type Input = SupersedeReferralInput;
    type Output = SupersedeReferralOutput;
    const ROUTE: &'static str = "/api/referral-attributions/{id}/supersede";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        // An unknown capture is not ACTIVE: the service answers `staleVersion` for it.
        let capture = CaptureId::new(
            Uuid::parse_str(&input.id).map_err(|_| capture_refusal(CaptureError::NotActive))?,
        );
        let owner: Option<String> =
            sqlx::query_scalar("SELECT branch_id FROM referral_attributions WHERE id = $1")
                .bind(capture.as_uuid())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "supersedeReferral: read the capture's branch");
                    server_error()
                })?;
        if owner.is_some_and(|b| b != branch.as_str()) {
            return Err(out_of_scope());
        }
        if input.reason.trim().is_empty() {
            return Err(capture_refusal(CaptureError::EmptyReason));
        }
        let code = ReferralCode::parse(&input.referral_code).map_err(capture_refusal)?;
        let id = self
            .captures
            .supersede_in(
                tx,
                &capture,
                code,
                &input.reason,
                who.id(),
                chrono::Utc::now(),
            )
            .await
            .map_err(capture_refusal)?;
        Ok(SupersedeReferralOutput {
            capture_id: id.to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(SupersedeReferralCommand {
            captures: capture_service(state),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    SupersedeReferralInput::export_all(&cfg)?;
    SupersedeReferralOutput::export_all(&cfg)?;
    Ok(())
}
