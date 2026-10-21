//! `POST /api/public/copy-link` (`copyLinkView`, design `kiosk-shared-phone.md` §4.2 D22, §4.4
//! B24, B25): the page of the 1-day link `<origin>/c#<link>` posts the fragment here. Public, no
//! session; read-only: it cannot cancel, reissue or sign in.
//!
//! `[required]` wire: body `{link}`; `200 {subject, id, branch, rows: [{field, value}], status,
//! qr, expiresAt}` (the `CopyView`; `qr` = the QR from the link, for the page to draw); every
//! failure — a bad shape, a forged or expired link, a reissued QR, a purged request — answers
//! `410 {error: {code: "linkExpired", …}}`, one answer. Rate limit by client address with today's
//! `QuoteLimit` shape (`xc-kiosk` `value_objects/quote_limit.rs`): over the limit ⇒ `429
//! tooManyRequests`. `Cache-Control: no-store`.

use actix_web::{HttpRequest, HttpResponse, web};

use crate::bootstrap::AppState;

pub const ROUTE: &str = "/api/public/copy-link";

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone()))
        .route(ROUTE, web::post().to(handle));
}

async fn handle(req: HttpRequest, state: web::Data<AppState>, body: web::Bytes) -> HttpResponse {
    let _ = (req, state, body);
    todo!("KWQ-D8: rate limit, CopyByEmailService::view_link, one 410")
}
