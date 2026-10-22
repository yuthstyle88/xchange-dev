//! `webRequestQrReissue` (`POST /api/app/web-requests/{id}/qr`, design `kiosk-shared-phone.md`
//! §4.2 D3; §4.3a W3; §4.4 B12): the member's own request gets a fresh request token; the old
//! one then leads nowhere (the queue machine, the desk, `copyByEmail`, old links).
//!
//! `[required]` wire: input `{branch, id}` (`id` from the path); `200 {requestId, requestQrUrl,
//! expiresAt}` exactly as `webRequestCreate` answers (`WebRequestAnswer`; the idempotency record
//! keeps `requestQrUrl: null`). Not the member's request, unknown, or no longer `QUOTED`/`WAITING`
//! ⇒ `404 notFound`, one answer. Signed out ⇒ `401 signIn`.

use std::path::Path;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use pos_error::AppError;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kiosk::application::kiosk::web_request::WebRequestService;

use super::web_request_create::WebRequestAnswer;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::kiosk::web_request_wiring::web_request_service;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRequestRef {
    pub branch: BranchId,
    pub id: String,
}

pub struct WebRequestQrReissue(pub WebRequestService);

impl CommandAction for WebRequestQrReissue {
    const NAME: &'static str = "webRequestQrReissue";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "webRequestQrReissue");
    type Input = WebRequestRef;
    type Output = WebRequestAnswer;
    const ROUTE: &'static str = "/api/app/web-requests/{id}/qr";
    const METHOD: Method = Method::POST;

    fn stored_output(&self, output: &Self::Output) -> Value {
        let mut val = serde_json::to_value(output).unwrap_or_default();
        val["requestQrUrl"] = Value::Null;
        val
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let customer_id = who.customer_id.as_deref().ok_or_else(out_of_scope)?;
        let token = Uuid::new_v4().to_string();
        let expires_at = self
            .0
            .reissue_qr_in(tx, branch, customer_id, &input.id, &token, Utc::now())
            .await
            .map_err(|err| match err {
                AppError::NotFound { .. } => {
                    V3Error::new("notFound", None, "notFound", StatusCode::NOT_FOUND)
                }
                other => {
                    tracing::error!(error = %other, "webRequestQrReissue failed");
                    server_error()
                }
            })?;
        Ok(WebRequestAnswer {
            request_id: input.id,
            request_qr_url: Some(format!("/q#{token}")),
            expires_at: expires_at.to_rfc3339(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        std::sync::Arc::new(WebRequestQrReissue(web_request_service(&state.db))),
    );
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
