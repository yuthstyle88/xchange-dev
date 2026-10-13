//! RV-2 (#682): `POST /api/transactions/{id}/reversal-requests` (design `reversal.md` §4.2) on the CM-1 command
//! seam, gated by `(Rpc, "reversalRequest")` (`ReversalRequest`, #874). Flat file: `build.rs` serves only plain
//! files in `commands/` (RG7).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use pos_error::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_kernel::ids::BranchId;

use super::commands::make_approval_service;
use crate::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};
use crate::application::workflows::save_step::SaveActor;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::reversal::ReversalDto;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, role_not_allowed_here, server_error,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReversalRequestInput {
    /// The path's `{id}`: the exchange to reverse.
    pub id: String,
    pub reason: String,
}

pub struct ReversalRequestCommand {
    service: RequestReversal,
}

/// A workflow refusal (`Err(AppError::Domain("<code>: …"))`) on the wire, with `approval_decide.rs`'s statuses.
fn refusal(e: AppError) -> V3Error {
    let AppError::Domain(m) = e else {
        tracing::error!(error = %e, "reversal request failed");
        return server_error();
    };
    let code = m.split(':').next().unwrap_or("").trim();
    let (code, status) = match code {
        "outOfScope" => ("outOfScope", StatusCode::FORBIDDEN),
        "staleVersion" => ("staleVersion", StatusCode::CONFLICT),
        "reversalWindowClosed" => ("reversalWindowClosed", StatusCode::CONFLICT),
        "idempotencyConflict" => ("idempotencyConflict", StatusCode::CONFLICT),
        _ => ("validationFailed", StatusCode::BAD_REQUEST),
    };
    V3Error::new(code, None, m, status)
}

impl CommandAction for ReversalRequestCommand {
    const NAME: &'static str = "reversalRequest";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "reversalRequest");
    type Input = ReversalRequestInput;
    type Output = ReversalDto;
    const ROUTE: &'static str = "/api/transactions/{id}/reversal-requests";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        // a malformed id answers as an unknown exchange: no existence is told
        let Ok(original) = Uuid::parse_str(&input.id) else {
            return Err(refusal(AppError::Domain(
                "outOfScope: no such exchange at this branch".into(),
            )));
        };
        let actor = SaveActor::from_actor(who).ok_or_else(role_not_allowed_here)?;
        let reversal = self
            .service
            .run_in(
                tx,
                RequestReversalInput {
                    branch: branch.clone(),
                    original: TransactionId(original),
                    reason: input.reason,
                    key: key.as_str().to_owned(),
                },
                &actor,
                Utc::now(),
            )
            .await
            .map_err(refusal)?;
        Ok(ReversalDto::from_domain(&reversal))
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    CommandEndpoint::register(
        cfg,
        Arc::new(ReversalRequestCommand {
            service: RequestReversal::new(pool.clone(), make_approval_service(pool)),
        }),
    );
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
