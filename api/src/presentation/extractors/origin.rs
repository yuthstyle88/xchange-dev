//! The same-origin rule of every route that applies it (ticket #161, v2's rule).
//!
//! The one function every route that applies the rule uses, with `actor.rs` re-exporting it:
//! - Header `X-Xchange-Request` is required and must be exactly `1`.
//! - Header `Origin` is required and compared byte-for-byte as sent.
//! - Scheme is `https` when `X-Forwarded-Proto` is exactly `https`, else `http`.
//! - Expected origin's `Host` is lower-cased with the scheme's default port (`:80` for `http`,
//!   `:443` for `https`) removed.
//! - A request with no `Host` is refused.

use actix_web::HttpRequest;
use actix_web::http::header;

fn expected_origin(req: &HttpRequest) -> Option<String> {
    let host_hdr = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())?;

    let scheme = if req
        .headers()
        .get("x-forwarded-proto")
        .is_some_and(|v| v.as_bytes() == b"https")
    {
        "https"
    } else {
        "http"
    };

    let host_lower = host_hdr.to_ascii_lowercase();
    let host = if scheme == "http" {
        host_lower.strip_suffix(":80").unwrap_or(&host_lower)
    } else {
        host_lower.strip_suffix(":443").unwrap_or(&host_lower)
    };

    Some(format!("{scheme}://{host}"))
}

/// v2's check of a write from the kept pages: same origin, and the marker header.
pub fn same_origin(req: &HttpRequest) -> bool {
    if !req
        .headers()
        .get("x-xchange-request")
        .is_some_and(|v| v.as_bytes() == b"1")
    {
        return false;
    }

    let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };

    let Some(expected) = expected_origin(req) else {
        return false;
    };

    origin == expected
}
