//! `webRequestCancel` (`POST /api/app/web-requests/{id}/cancel`, design `kiosk-shared-phone.md`
//! §4.2 D8; §4.4 B11a; owner R-11: one press, no approval): the member cancels their own request.
//!
//! `[required]` wire: input `{branch, id}`; `200 {requestId, status: "CANCELLED"}`. Not the
//! member's request or unknown ⇒ `404 notFound`; a request that can no longer be cancelled (at the
//! counter, done) ⇒ `409 notCancellable`. Signed out ⇒ `401 signIn`.

use std::path::Path;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kiosk::application::kiosk::web_request::WebRequestService;

use super::web_request_qr::WebRequestRef;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::kiosk::web_request_wiring::web_request_service;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRequestCancelled {
    pub request_id: String,
    pub status: String,
}

pub struct WebRequestCancel(pub WebRequestService);

impl CommandAction for WebRequestCancel {
    const NAME: &'static str = "webRequestCancel";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "webRequestCancel");
    type Input = WebRequestRef;
    type Output = WebRequestCancelled;
    const ROUTE: &'static str = "/api/app/web-requests/{id}/cancel";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (tx, branch, who, input, key, &self.0);
        todo!("KWQ-C7: WebRequestService::cancel")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        std::sync::Arc::new(WebRequestCancel(web_request_service(&state.db))),
    );
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
