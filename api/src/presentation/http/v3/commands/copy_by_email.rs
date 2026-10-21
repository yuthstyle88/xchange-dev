//! `POST /api/app/copy-by-email` (`copyByEmail`, design `kiosk-shared-phone.md` §4.2 D21, §4.4
//! B19–B23): a signed-in member has the copy of their own booking or web request e-mailed to their
//! verified e-mail. `CustomerSelf` (policy area `copy_by_email.rs`).
//!
//! A plain route, not a `CommandAction`: the link in the e-mail opens on the request's own origin,
//! which only the request carries (`Origin`, already checked by `same_origin`), and the send is
//! made idempotent by Notify's key, not by a stored command row.
//!
//! `[required]` wire: body `{subject: "booking" | "webRequest", id, qr}`; answers `200 {sent:
//! true}` or `200 {sent: true, again: true}`; `404 notFound`, `409 noVerifiedEmail`, `503
//! emailUnavailable` in the V3 error envelope (`{error: {code, …}}`); an unknown `subject`, or a
//! missing/empty field ⇒ `400 validationFailed` naming the field. The body has no address field:
//! the recipient is never the caller's choice.

use std::path::Path;

use actix_web::{HttpRequest, HttpResponse, web};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::bootstrap::AppState;

pub const ROUTE: &str = "/api/app/copy-by-email";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "copy_by_email.ts")]
pub struct CopyByEmailInput {
    /// `booking` or `webRequest`.
    pub subject: String,
    pub id: String,
    /// The QR in the clear, as the member's own page holds it (the part after `#`).
    pub qr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "copy_by_email.ts")]
pub struct CopyByEmailOutput {
    pub sent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub again: Option<bool>,
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone()))
        .route(ROUTE, web::post().to(handle));
}

async fn handle(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Bytes,
) -> HttpResponse {
    let _ = (req, state, body);
    todo!("KWQ-D7: same_origin, authorize copyByEmail, CopyByEmailService::send")
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CopyByEmailInput::export_all(&cfg)?;
    CopyByEmailOutput::export_all(&cfg)?;
    Ok(())
}
