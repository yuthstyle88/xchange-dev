//! Guard test (#2048): every browser write route under `api/src` must enforce
//! `same_origin(`, or appear on the explicit allowlist with a non-empty reason.

use std::fs;
use std::path::{Path, PathBuf};

const WRITE_MARKERS: &[&str] = &[
    "web::post()",
    "web::put()",
    "web::patch()",
    "web::delete()",
    "#[post(",
    "#[put(",
    "#[patch(",
    "#[delete(",
    "web::method(",
];

const V3_CENTRAL_FILES: &[&str] = &[
    "src/presentation/http/v3/command.rs",
    "src/presentation/http/v3/action.rs",
];

const ALLOWLIST: &[(&str, &str)] = &[(
    "src/presentation/http/device_agent.rs",
    "machine endpoint with no cookie session, browsers refused by check_browser in agent_auth.rs:42, authenticated by bearer token or pairing code",
)];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScanReport {
    missing_same_origin: Vec<String>,
    stale_allowlist: Vec<String>,
    empty_allowlist_reasons: Vec<String>,
}

impl ScanReport {
    fn is_ok(&self) -> bool {
        self.missing_same_origin.is_empty()
            && self.stale_allowlist.is_empty()
            && self.empty_allowlist_reasons.is_empty()
    }
}

/// Strips `//` line comments (including trailing comments to end-of-line) and
/// `/* ... */` block comments (including nested block comments), while preserving
/// string literals (`"..."` and raw `r#"..."#`) so `//` or `/*` inside a string
/// literal does not swallow code that follows on the same line.
fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;

    while i < len {
        // 1. Line comment: `//` to end of line
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // 2. Block comment: `/* ... */` (supports nested block comments)
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1usize;
            while i < len && depth > 0 {
                if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if bytes[i] == b'\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }

        // 3. Raw string literal: `r"..."` or `r#"..."#`
        if bytes[i] == b'r'
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
        {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < len && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < len && bytes[j] == b'"' {
                j += 1;
                while j < len {
                    if bytes[j] == b'"'
                        && j + 1 + hashes <= len
                        && bytes[j + 1..j + 1 + hashes].iter().all(|&b| b == b'#')
                    {
                        j += 1 + hashes;
                        break;
                    }
                    j += 1;
                }
                out.push_str(&source[i..j]);
                i = j;
                continue;
            }
        }

        // 4. Standard string literal: `"..."`
        if bytes[i] == b'"' {
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            out.push_str(&source[start..i]);
            continue;
        }

        // 5. Character literal (`'x'` or `'\x'`), distinct from lifetimes (`'a`)
        if bytes[i] == b'\'' {
            if i + 3 < len && bytes[i + 1] == b'\\' && bytes[i + 3] == b'\'' {
                out.push_str(&source[i..i + 4]);
                i += 4;
                continue;
            }
            if i + 2 < len && bytes[i + 1] != b'\'' && bytes[i + 2] == b'\'' {
                out.push_str(&source[i..i + 3]);
                i += 3;
                continue;
            }
        }

        let ch = source[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn registers_write(code_without_comments: &str) -> bool {
    WRITE_MARKERS
        .iter()
        .any(|marker| code_without_comments.contains(marker))
}

fn has_same_origin(code_without_comments: &str) -> bool {
    code_without_comments.contains("same_origin(")
}

fn scan_sources(files: &[(&str, &str)], allowlist: &[(&str, &str)]) -> ScanReport {
    let mut missing_same_origin = Vec::new();
    let mut stale_allowlist = Vec::new();
    let mut empty_allowlist_reasons = Vec::new();

    for (allowed_path, reason) in allowlist {
        if reason.trim().is_empty() {
            empty_allowlist_reasons.push((*allowed_path).to_string());
        }
    }

    for (path, source) in files {
        let code = strip_comments(source);
        if !registers_write(&code) {
            continue;
        }
        let is_allowlisted = allowlist
            .iter()
            .any(|(allowed_path, _)| allowed_path == path);
        if !has_same_origin(&code) && !is_allowlisted {
            missing_same_origin.push((*path).to_string());
        }
    }

    for (allowed_path, _) in allowlist {
        let still_needed = files
            .iter()
            .find(|(path, _)| path == allowed_path)
            .is_some_and(|(_, source)| {
                let code = strip_comments(source);
                registers_write(&code) && !has_same_origin(&code)
            });
        if !still_needed {
            stale_allowlist.push((*allowed_path).to_string());
        }
    }

    missing_same_origin.sort();
    stale_allowlist.sort();
    empty_allowlist_reasons.sort();

    ScanReport {
        missing_same_origin,
        stale_allowlist,
        empty_allowlist_reasons,
    }
}

fn check_v3_central(files: &[(&str, &str)]) -> Vec<String> {
    let mut missing = Vec::new();
    for required in V3_CENTRAL_FILES {
        let ok = files
            .iter()
            .find(|(path, _)| path == required)
            .is_some_and(|(_, source)| has_same_origin(&strip_comments(source)));
        if !ok {
            missing.push((*required).to_string());
        }
    }
    missing
}

fn api_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn load_api_src_files() -> Vec<(String, String)> {
    let root = api_dir();
    let src_dir = root.join("src");
    let mut paths = Vec::new();
    collect_rs_files(&src_dir, &mut paths);
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            (rel, content)
        })
        .collect()
}

#[test]
fn write_routes_in_api_src_require_same_origin_or_allowlist() {
    let loaded = load_api_src_files();
    let borrowed: Vec<(&str, &str)> = loaded
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_str()))
        .collect();

    let missing_central = check_v3_central(&borrowed);
    let report = scan_sources(&borrowed, ALLOWLIST);

    assert!(
        missing_central.is_empty() && report.is_ok(),
        "write-route same_origin guard failed:\n  v3 central check missing same_origin(: {:?}\n  write route files missing same_origin(: {:?}\n  stale allowlist entries: {:?}\n  empty allowlist reasons: {:?}",
        missing_central,
        report.missing_same_origin,
        report.stale_allowlist,
        report.empty_allowlist_reasons,
    );
}

#[test]
fn fixture_1_web_post_without_same_origin_is_flagged() {
    let files = [(
        "src/presentation/http/unguarded.rs",
        r#"
        pub fn configure(cfg: &mut web::ServiceConfig) {
            cfg.route("/api/example", web::post().to(handler));
        }
        "#,
    )];
    let report = scan_sources(&files, &[]);
    assert_eq!(
        report.missing_same_origin,
        vec!["src/presentation/http/unguarded.rs".to_string()]
    );
    assert!(report.stale_allowlist.is_empty());
}

#[test]
fn fixture_2_web_post_with_same_origin_passes() {
    let files = [(
        "src/presentation/http/guarded.rs",
        r#"
        pub fn configure(cfg: &mut web::ServiceConfig) {
            cfg.route("/api/example", web::post().to(handler));
        }
        async fn handler(req: HttpRequest) -> HttpResponse {
            if !same_origin(&req) {
                return refuse();
            }
            ok()
        }
        "#,
    )];
    let report = scan_sources(&files, &[]);
    assert!(report.is_ok(), "expected clean report, got: {report:?}");
}

#[test]
fn fixture_3_web_post_only_in_comment_line_passes() {
    let files = [(
        "src/presentation/http/comment_only.rs",
        r#"
        // Example route: cfg.route("/api/example", web::post().to(handler));
        pub fn helper() -> usize {
            42
        }
        "#,
    )];
    let report = scan_sources(&files, &[]);
    assert!(report.is_ok(), "expected clean report, got: {report:?}");
}

#[test]
fn fixture_4_allowlisted_file_passes() {
    let files = [(
        "src/presentation/http/device_agent.rs",
        r#"
        pub fn configure(cfg: &mut web::ServiceConfig) {
            cfg.route("/api/device-agent/pair", web::post().to(pair_handler));
        }
        "#,
    )];
    let report = scan_sources(&files, ALLOWLIST);
    assert!(report.is_ok(), "expected clean report, got: {report:?}");
}

#[test]
fn fixture_5_stale_allowlist_entry_is_flagged() {
    let files = [(
        "src/presentation/http/device_agent.rs",
        r#"
        pub fn configure(cfg: &mut web::ServiceConfig) {
            cfg.route("/api/device-agent/status", web::get().to(status_handler));
        }
        "#,
    )];
    let report = scan_sources(&files, ALLOWLIST);
    assert!(report.missing_same_origin.is_empty());
    assert_eq!(
        report.stale_allowlist,
        vec!["src/presentation/http/device_agent.rs".to_string()]
    );
}

#[test]
fn fixture_6_attribute_post_handler_without_check_is_flagged() {
    let files = [(
        "src/presentation/http/attr_route.rs",
        r#"
        #[post("/x")]
        async fn handle_x() -> impl Responder {
            HttpResponse::Ok()
        }
        "#,
    )];
    let report = scan_sources(&files, &[]);
    assert_eq!(
        report.missing_same_origin,
        vec!["src/presentation/http/attr_route.rs".to_string()]
    );
    assert!(report.stale_allowlist.is_empty());
}

#[test]
fn fixture_7_write_route_with_only_whole_line_comment_same_origin_is_flagged() {
    let files = [(
        "src/presentation/http/whole_line_comment.rs",
        r#"
        pub fn configure(cfg: &mut web::ServiceConfig) {
            cfg.route("/api/example", web::post().to(handler));
        }
        async fn handler(req: HttpRequest) -> HttpResponse {
            // if !same_origin(&req) { return refuse(); }
            ok()
        }
        "#,
    )];
    let report = scan_sources(&files, &[]);
    assert_eq!(
        report.missing_same_origin,
        vec!["src/presentation/http/whole_line_comment.rs".to_string()]
    );
}

#[test]
fn fixture_8_write_route_with_only_trailing_comment_same_origin_is_flagged() {
    let files = [(
        "src/presentation/http/trailing_comment.rs",
        r#"
        pub fn configure(cfg: &mut web::ServiceConfig) {
            cfg.route("/api/example", web::post().to(handler));
        }
        async fn handler(req: HttpRequest) -> HttpResponse {
            if false { // same_origin(&req)
                return refuse();
            }
            ok()
        }
        "#,
    )];
    let report = scan_sources(&files, &[]);
    assert_eq!(
        report.missing_same_origin,
        vec!["src/presentation/http/trailing_comment.rs".to_string()]
    );
}

#[test]
fn fixture_9_write_route_with_only_block_comment_same_origin_is_flagged() {
    let files = [(
        "src/presentation/http/block_comment.rs",
        r#"
        pub fn configure(cfg: &mut web::ServiceConfig) {
            cfg.route("/api/example", web::post().to(handler));
        }
        async fn handler(req: HttpRequest) -> HttpResponse {
            /* same_origin(&req) */
            ok()
        }
        "#,
    )];
    let report = scan_sources(&files, &[]);
    assert_eq!(
        report.missing_same_origin,
        vec!["src/presentation/http/block_comment.rs".to_string()]
    );
}

#[test]
fn fixture_10_double_slash_inside_string_literal_does_not_hide_same_origin() {
    let files = [(
        "src/presentation/http/string_literal_url.rs",
        r#"
        pub fn configure(cfg: &mut web::ServiceConfig) {
            cfg.route("/api/example", web::post().to(handler));
        }
        async fn handler(req: HttpRequest) -> HttpResponse {
            let _doc = "https://example.com/spec"; if !same_origin(&req) { return refuse(); }
            ok()
        }
        "#,
    )];
    let report = scan_sources(&files, &[]);
    assert!(report.is_ok(), "expected clean report, got: {report:?}");
}
