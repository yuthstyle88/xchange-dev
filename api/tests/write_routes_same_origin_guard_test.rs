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

fn strip_comment_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
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
        let code = strip_comment_lines(source);
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
                let code = strip_comment_lines(source);
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
            .is_some_and(|(_, source)| has_same_origin(&strip_comment_lines(source)));
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
