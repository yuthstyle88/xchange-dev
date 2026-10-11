//! The anonymous app session guard (design P7, §4.3; ticket #994 §7, §8).
//!
//! The session is the `xc_app_sid` cookie (`HttpOnly; Secure; SameSite=Lax`), issued by the first
//! app request that needs it; only its SHA-256 is stored. The guard also limits the session's draft
//! writes: at most [`DRAFT_WRITES_PER_MINUTE`] in any one minute, then `rateLimited` (429).

use std::collections::{HashMap, VecDeque};
use std::future::{Ready, ready};
use std::sync::Mutex;

use actix_web::cookie::{Cookie, SameSite};
use actix_web::dev::Payload;
use actix_web::http::StatusCode;
use actix_web::{FromRequest, HttpRequest};
use chrono::{DateTime, Duration, Utc};
use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSession, AppSessionId, SessionHash};

use crate::presentation::http::v3::error::V3Error;

/// [proposed, look-only] N of ticket #994 §8: draft writes per session per minute.
pub const DRAFT_WRITES_PER_MINUTE: u32 = 20;

/// The `rateLimited` sentence (Thai, as every v3 refusal).
const RATE_LIMITED: &str = "ทำรายการถี่เกินไป กรุณารอสักครู่แล้วลองใหม่";

/// The request's app session: read from `xc_app_sid`, or a new one when the request carries none
/// (or one `AppSessionId::parse` refuses).
#[derive(Debug, Clone)]
pub struct AppSessionCookie {
    session: AppSession,
    issued: bool,
}

impl AppSessionCookie {
    pub fn session(&self) -> &AppSession {
        &self.session
    }

    /// The `Set-Cookie` to answer with when this request was given a new id ([`session_cookie`]);
    /// `None` when the request carried a valid one.
    pub fn issued_cookie(&self) -> Option<Cookie<'static>> {
        self.issued.then(|| session_cookie(&self.session.id))
    }

    fn of_request(req: &HttpRequest) -> Self {
        let carried = req
            .cookie(APP_SESSION_COOKIE)
            .and_then(|c| AppSessionId::parse(c.value()));
        let (id, issued) = match carried {
            Some(id) => (id, false),
            None => (AppSessionId::generate(), true),
        };
        let hash = id.hash();
        Self {
            session: AppSession { id, hash },
            issued,
        }
    }
}

impl FromRequest for AppSessionCookie {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        ready(Ok(Self::of_request(req)))
    }
}

/// The cookie that carries `id`: `xc_app_sid`, `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`.
pub fn session_cookie(id: &AppSessionId) -> Cookie<'static> {
    Cookie::build(APP_SESSION_COOKIE, id.as_str().to_owned())
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .path("/")
        .finish()
}

/// Limits each session's draft writes (shared app data, one per process).
pub struct AppSessionGuard {
    per_minute: u32,
    /// The times of each session's writes within the last minute.
    writes: Mutex<HashMap<SessionHash, VecDeque<DateTime<Utc>>>>,
}

impl AppSessionGuard {
    pub fn new(per_minute: u32) -> Self {
        Self {
            per_minute,
            writes: Mutex::new(HashMap::new()),
        }
    }

    /// Counts one draft write of `session` at `now`. More than `per_minute` writes of one session
    /// within 60 s → `rateLimited` (429); other sessions are not affected.
    pub fn admit_draft_write(
        &self,
        session: &SessionHash,
        now: DateTime<Utc>,
    ) -> Result<(), V3Error> {
        let window_start = now - Duration::seconds(60);
        let mut writes = self
            .writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Forget sessions with no write in the window, so the map does not grow without bound.
        writes.retain(|_, times| times.back().is_some_and(|t| *t > window_start));
        let times = writes.entry(session.clone()).or_default();
        while times.front().is_some_and(|t| *t <= window_start) {
            times.pop_front();
        }
        if times.len() >= self.per_minute as usize {
            return Err(V3Error::new(
                "rateLimited",
                None,
                RATE_LIMITED,
                StatusCode::TOO_MANY_REQUESTS,
            ));
        }
        times.push_back(now);
        Ok(())
    }
}
