//! `webRequestCreate` (`POST /api/app/web-requests`, design `kiosk-shared-phone.md` §4.2 D2, D12;
//! §4.3a W1, W3; §4.4 B2–B4): a signed-in member's request from the web page — a `QUOTED`
//! request with the member and a request token, **no ticket and no number**.
//!
//! `[required]` wire (amends W1/W3, see the slice-c note on #564):
//! - Input `{branch, type: "BUY"|"SELL", currency, amount: number|string, paymentMethod?}`;
//!   `branch` is the branch **id** the v3 seam authorises (as `queueTicketTake`). `type`,
//!   `currency`, `amount`, `paymentMethod` are parsed exactly as the kiosk `quote` parses them
//!   (`presentation/http/kiosk.rs`, `KioskAction::Quote`).
//! - `200 {requestId, requestQrUrl, expiresAt}`: `requestQrUrl` = `/q#<token>` (relative; the page
//!   prefixes its own origin), `expiresAt` RFC 3339. The token is a fresh UUID v4, in the clear
//!   only in this answer; the idempotency record keeps `requestQrUrl: null` (as `bookingQrReissue`).
//! - Signed out ⇒ the seam's `401 signIn` (B2). The member's open request at the branch ⇒
//!   `409 alreadyQueued`, message = its id (B4). A refused quote (the kiosk's sentence) ⇒
//!   `400 quoteRefused`, message = the sentence. Unknown branch ⇒ the seam's answer.

use std::path::Path;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kiosk::application::kiosk::web_request::WebRequestService;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::kiosk::web_request_wiring::web_request_service;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRequestCreateInput {
    pub branch: BranchId,
    #[serde(rename = "type")]
    pub side: Option<String>,
    pub currency: Option<String>,
    pub amount: Option<Value>,
    pub payment_method: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRequestAnswer {
    pub request_id: String,
    pub request_qr_url: Option<String>,
    pub expires_at: String,
}

pub struct WebRequestCreate(pub WebRequestService);

impl CommandAction for WebRequestCreate {
    const NAME: &'static str = "webRequestCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "webRequestCreate");
    type Input = WebRequestCreateInput;
    type Output = WebRequestAnswer;
    const ROUTE: &'static str = "/api/app/web-requests";
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
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (tx, branch, who, input, key, &self.0);
        todo!("KWQ-C5: parse as the kiosk quote, mint the token, WebRequestService::create")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        std::sync::Arc::new(WebRequestCreate(web_request_service(&state.db))),
    );
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
