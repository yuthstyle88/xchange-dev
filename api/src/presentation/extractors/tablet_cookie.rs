//! The branch tablet's cookie (design `kiosk-shared-phone.md` §7.3 T-D3, §7.7): reading it and
//! the two `Set-Cookie` lines that write and clear it.
//!
//! The cookie is not an `Actor`: `Policy` never sees it and it opens no staff or customer route.
//! Only `GET /api/branch-tablet/me` and the sign-in binding read it.
//!
//! Filled by BT-4. `api/tests/branch_tablet_http_test.rs` pins the attributes. The sign-in
//! binding (BT-5, `auth.rs`) reads the cookie with `extractors::actor::cookie(req, TABLET_COOKIE)`
//! so that it does not wait for this file.

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponseBuilder};

pub use crate::infrastructure::branch_tablets::{TABLET_COOKIE, TABLET_COOKIE_MAX_AGE};
use crate::presentation::extractors::actor::cookie;

const ATTRIBUTES: &str = "Path=/; HttpOnly; Secure; SameSite=Strict";

/// The tablet cookie's value, trimmed; `None` when the request carries none (or an empty one).
pub fn tablet_token(req: &HttpRequest) -> Option<String> {
    let raw = cookie(req, TABLET_COOKIE);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Appends `Set-Cookie: __Host-xc_tablet=<token>; Path=/; HttpOnly; Secure; SameSite=Strict;
/// Max-Age=34560000`.
pub fn set_tablet_cookie(builder: &mut HttpResponseBuilder, token: &str) {
    builder.append_header((
        header::SET_COOKIE,
        format!("{TABLET_COOKIE}={token}; {ATTRIBUTES}; Max-Age={TABLET_COOKIE_MAX_AGE}"),
    ));
}

/// Appends `Set-Cookie: __Host-xc_tablet=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0`.
pub fn clear_tablet_cookie(builder: &mut HttpResponseBuilder) {
    builder.append_header((
        header::SET_COOKIE,
        format!("{TABLET_COOKIE}=; {ATTRIBUTES}; Max-Age=0"),
    ));
}
