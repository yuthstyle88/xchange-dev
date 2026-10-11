//! The request's own host, as the browser sent it (ticket #1255 RD-0, `docs/design/blueprint/rate-display.md`
//! §3, §4.1a).

use actix_web::HttpRequest;
use actix_web::http::header;

/// `Host`, lower-cased, with the scheme's default port removed (`:80` for `http`, `:443` for `https`, the
/// scheme being `https` exactly when `X-Forwarded-Proto` is `https`). A non-default port is kept.
/// `X-Forwarded-Host` is never read. `None` when the request has no `Host`.
pub fn request_host(req: &HttpRequest) -> Option<String> {
    let host_hdr = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())?;

    let is_https = req
        .headers()
        .get("x-forwarded-proto")
        .is_some_and(|v| v.as_bytes() == b"https");

    let host_lower = host_hdr.to_ascii_lowercase();
    let host = if is_https {
        host_lower.strip_suffix(":443").unwrap_or(&host_lower)
    } else {
        host_lower.strip_suffix(":80").unwrap_or(&host_lower)
    };

    Some(host.to_string())
}
