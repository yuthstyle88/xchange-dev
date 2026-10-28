use actix_web::http::header::{self, HeaderMap};

/// RFC 7232 §3.2 weak ETag comparison for `If-None-Match`.
pub fn if_none_match_hits(headers: &HeaderMap, etag: &str) -> bool {
    let target = etag.trim().strip_prefix("W/").unwrap_or(etag.trim());
    headers
        .get_all(header::IF_NONE_MATCH)
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(','))
        .any(|part| {
            let part = part.trim();
            part == "*" || part.strip_prefix("W/").unwrap_or(part) == target
        })
}
