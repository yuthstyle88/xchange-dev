//! The caller's address, as every per-address limit of the server keys on it.
//!
//! The server runs behind a proxy, and only the proxy knows who is calling. It says so in one
//! header, `X-Real-IP`, which it sets on every request and a client therefore cannot choose.
//! `CF-Connecting-IP` and `X-Forwarded-For` reach the server as the client wrote them unless
//! the proxy clears them, so the server never reads either (ticket #130).
//!
//! This is the one place that reads the address. The kiosk quote limit uses it, and so does every
//! per-address limit; none reads the request's headers itself
//! (`tests/client_address_fn_test.rs` holds the sources to that and pins the function itself;
//! `tests/or7m_w2_client_address_test.rs` pins the address rules on the kiosk quote limit).
//!
//! Known difference from v2: v2's staff door read `CF-Connecting-IP` first and `X-Real-IP`
//! only without it (the `addresses` story of v2's auth fixture, deleted in OR-7.m w2 #1027).
//! v3 reads `X-Real-IP` wherever it reads an address.

use actix_web::HttpRequest;

/// The one request header the server reads the caller's address from.
pub const CLIENT_ADDRESS_HEADER: &str = "x-real-ip";

/// The address of a caller whose request names none. All such callers share one counter.
pub const UNKNOWN_ADDRESS: &str = "unknown";

/// The caller's address: what `X-Real-IP` says, without the spaces around it.
///
/// [`UNKNOWN_ADDRESS`] when the header is absent, empty, blank, or not visible ASCII.
/// It never fails and never panics; the value is not checked to be an IP address, because
/// the limits only need callers told apart, and the proxy is trusted to write it.
pub fn client_address(req: &HttpRequest) -> &str {
    req.headers()
        .get(CLIENT_ADDRESS_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .unwrap_or(UNKNOWN_ADDRESS)
}
