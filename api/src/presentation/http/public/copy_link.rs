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

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::Utc;
use serde_json::{Value, json};
use xc_kiosk::domain::kiosk::value_objects::quote_limit::QuoteLimit;

use crate::application::copy_by_email::booking_copy::BookingCopy;
use crate::application::copy_by_email::link::CopyLink;
use crate::application::copy_by_email::member_email::PgMemberEmails;
use crate::application::copy_by_email::service::CopyByEmailService;
use crate::application::copy_by_email::subject::CopySubject;
use crate::application::copy_by_email::web_request_copy::WebRequestCopy;
use crate::bootstrap::AppState;
use crate::infrastructure::notify::notify_email_sender::NotifyEmailSender;
use crate::presentation::http::client_address::client_address;
use crate::presentation::http::v3::error::V3Error;

pub const ROUTE: &str = "/api/public/copy-link";

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone()))
        .route(ROUTE, web::post().to(handle));
}

fn rate_map() -> &'static Mutex<HashMap<String, Vec<Instant>>> {
    static WINDOWS: OnceLock<Mutex<HashMap<String, Vec<Instant>>>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn rate_limited(address: &str) -> bool {
    rate_limited_at(address, Instant::now())
}

fn rate_limited_at(address: &str, now: Instant) -> bool {
    let mut map = rate_map().lock().unwrap_or_else(|e| e.into_inner());
    let window = Duration::from_millis(QuoteLimit::WINDOW_MS as u64);
    map.retain(|_, v| {
        v.retain(|&t| now.duration_since(t) < window);
        !v.is_empty()
    });
    let timestamps = map.entry(address.to_owned()).or_default();
    if QuoteLimit::reached(timestamps.len() as i64) {
        if timestamps.is_empty() {
            map.remove(address);
        }
        return true;
    }
    timestamps.push(now);
    false
}

fn link_expired() -> HttpResponse {
    V3Error::new("linkExpired", None, "ลิงก์หมดอายุหรือไม่ถูกต้อง", StatusCode::GONE).error_response()
}

fn too_many_requests() -> HttpResponse {
    V3Error::new(
        "tooManyRequests",
        None,
        "คำขอมากเกินไป กรุณาลองใหม่ภายหลัง",
        StatusCode::TOO_MANY_REQUESTS,
    )
    .error_response()
}

async fn handle(req: HttpRequest, state: web::Data<AppState>, body: web::Bytes) -> HttpResponse {
    if rate_limited(client_address(&req)) {
        return too_many_requests();
    }

    let Ok(body_val) = serde_json::from_slice::<Value>(&body) else {
        return link_expired();
    };
    let Some(link) = body_val
        .get("link")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return link_expired();
    };

    let Ok(pii_keys) = xc_customer::get_pii_keys() else {
        return link_expired();
    };

    let subjects: Vec<Arc<dyn CopySubject>> = vec![
        Arc::new(BookingCopy::new(state.db.clone())),
        Arc::new(WebRequestCopy::new(state.db.clone())),
    ];
    let members = Arc::new(PgMemberEmails::new(state.db.clone(), pii_keys.clone()));
    let sender = Arc::new(NotifyEmailSender::from_env());
    let service = CopyByEmailService::new(
        subjects,
        members,
        sender,
        CopyLink::from_pii(&pii_keys),
        String::new(),
    );

    let Ok(view) = service.view_link(link, Utc::now()).await else {
        return link_expired();
    };

    let rows: Vec<Value> = view
        .details
        .rows
        .into_iter()
        .map(|(field, value)| json!({ "field": field, "value": value }))
        .collect();

    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(json!({
            "subject": view.details.kind.as_str(),
            "id": view.details.id,
            "branch": view.details.branch_name,
            "rows": rows,
            "status": view.details.status,
            "qr": view.qr,
            "expiresAt": view.expires_at.to_rfc3339(),
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_map_drops_address_after_window_passes() {
        let t0 = Instant::now();
        let window = Duration::from_millis(QuoteLimit::WINDOW_MS as u64);
        assert!(!rate_limited_at("198.51.100.250", t0));
        assert!(rate_map().lock().unwrap().contains_key("198.51.100.250"));

        assert!(!rate_limited_at("198.51.100.251", t0 + window));
        let map = rate_map().lock().unwrap();
        assert!(!map.contains_key("198.51.100.250"));
        assert!(map.contains_key("198.51.100.251"));
    }
}
