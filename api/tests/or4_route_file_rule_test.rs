//! OR-4b ruling (b), amended: a pure `CommandEndpoint` registrar file is exempt from the
//! INHERITS / PUBLIC_ROUTE marker, so it no longer has to borrow a public marker it does not mean.
//!
//! Written before the implementation by the ticket author (claude/d7aa64). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! The rule lives in the shared test module `tests/or4_route_rules/mod.rs` as
//! `route_file_verdict(code) -> Result<(), String>`, where `code` is the file text with its `//`
//! comment lines already removed (what `code_of` returns). A file passes when:
//!
//! - it declares `const INHERITS:` on an impl of BranchView/PagedView/ComputeAction/CommandAction/
//!   OrgView, or it declares `const PUBLIC_ROUTE:` (unchanged rule); or
//! - it is a pure registrar: at least one `CommandEndpoint::register(` call and no raw route
//!   (`.route(`, `web::resource(`, `web::get()`/`post()`/`put()`/`delete()`/`patch()`,
//!   `#[get(`/`#[post(`/`#[put(`/`#[delete(`/`#[patch(`, `.service(`).
//!
//! `INHERITS` declared off one of the five traits still fails. No database.

mod or4_route_rules;

use std::fs;
use std::path::{Path, PathBuf};

use or4_route_rules::route_file_verdict;

/// A pure `CommandEndpoint` registrar, the shape `commands/bookings.rs` takes after #1913.
const REGISTRAR: &str = r#"
use std::sync::Arc;

use actix_web::web;
use serde::Deserialize;

use crate::presentation::http::v3::command_endpoint::CommandEndpoint;

pub mod create;
pub mod cancel;

#[derive(Deserialize)]
pub struct Dto {
    pub id: String,
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(create::Create::new(state)));
    CommandEndpoint::register(cfg, Arc::new(cancel::Cancel::new(state)));
}
"#;

/// The text `code_of` hands the rule: every line starting with `//` (after indentation) dropped.
fn without_comment_lines(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn with(extra: &str) -> String {
    format!("{REGISTRAR}\n{extra}\n")
}

#[test]
fn row1_a_pure_command_endpoint_registrar_passes_without_a_marker() {
    assert!(!REGISTRAR.contains("const INHERITS:"));
    assert!(!REGISTRAR.contains("const PUBLIC_ROUTE:"));
    assert_eq!(route_file_verdict(REGISTRAR), Ok(()));
}

#[test]
fn row2_a_registrar_with_a_raw_cfg_route_fails() {
    let code = with(
        r#"pub fn raw(cfg: &mut web::ServiceConfig) {
    cfg.route("/api/x", web::post().to(handler));
}"#,
    );
    assert!(
        route_file_verdict(&code).is_err(),
        "raw .route( in a registrar"
    );
}

#[test]
fn row3_a_registrar_with_a_raw_service_resource_fails() {
    let code = with(
        r#"pub fn raw(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/api/x").route(web::get().to(h)));
}"#,
    );
    assert!(
        route_file_verdict(&code).is_err(),
        "raw .service(web::resource( in a registrar"
    );
    // `.service(` alone: an attribute handler defined elsewhere, mounted from the registrar
    let code = with(
        r#"pub fn raw(cfg: &mut web::ServiceConfig) {
    cfg.service(raw_handler);
}"#,
    );
    assert!(
        route_file_verdict(&code).is_err(),
        "raw .service(handler) in a registrar"
    );
}

#[test]
fn row4_a_registrar_with_a_route_attribute_handler_fails() {
    let code = with(
        r#"#[post("/api/x")]
async fn h() {}"#,
    );
    assert!(
        route_file_verdict(&code).is_err(),
        "#[post( handler in a registrar"
    );
}

#[test]
fn row5_a_raw_public_file_needs_its_own_marker() {
    let raw = r#"
use actix_web::web;

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(PUBLIC_ROUTE, web::post().to(handle));
}

async fn handle() -> &'static str {
    "ok"
}
"#;
    let with_marker = format!("pub const PUBLIC_ROUTE: &str = \"/api/kiosk/x\";\n{raw}");
    assert_eq!(
        route_file_verdict(&with_marker),
        Ok(()),
        "the kiosk file with its marker"
    );
    assert!(
        route_file_verdict(raw).is_err(),
        "the kiosk file without its PUBLIC_ROUTE const"
    );
}

#[test]
fn row6_inherits_on_a_command_action_impl_passes() {
    let code = r#"
pub struct X;

impl CommandAction for X {
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "x");
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/api/v3/commands/x", web::post().to(handle::<X>));
}
"#;
    assert_eq!(route_file_verdict(code), Ok(()));
}

#[test]
fn row7_inherits_off_the_five_traits_fails() {
    let code = r#"
pub struct X;

impl X {
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "x");
}
"#;
    assert!(
        route_file_verdict(code).is_err(),
        "INHERITS on an impl of no shared trait"
    );
}

#[test]
fn row8_a_file_with_no_register_call_and_no_marker_fails() {
    let code = r#"
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Dto {
    pub id: String,
}
"#;
    assert!(
        route_file_verdict(code).is_err(),
        "the exemption needs at least one CommandEndpoint::register("
    );
}

#[test]
fn row9_raw_route_text_on_a_comment_line_does_not_count() {
    let raw_text = format!(
        "{REGISTRAR}\n// cfg.route(\"/api/x\", web::post().to(handler));\n    // #[post(\"/api/x\")]\n"
    );
    let code = without_comment_lines(&raw_text);
    assert!(
        !code.contains(".route("),
        "the filter dropped the comment line"
    );
    assert_eq!(route_file_verdict(&code), Ok(()));
}

// ---- row 10: the real tree, read the way `or4_routes_guard_test.rs` reads it ----

fn api_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// The text of a file without its comment lines.
fn code_of(path: &Path) -> String {
    without_comment_lines(
        &fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
}

/// The files `build.rs` registers as v3 routes: every `.rs` under `views/`, `actions/` and
/// `commands/` but a `mod.rs`.
fn v3_route_files() -> Vec<PathBuf> {
    let v3 = api_dir().join("src/presentation/http/v3");
    let mut files = Vec::new();
    for dir in ["views", "actions", "commands"] {
        rust_files(&v3.join(dir), &mut files);
    }
    files.retain(|f| f.file_name().is_some_and(|n| n != "mod.rs"));
    files.sort();
    files
}

#[test]
fn row10_every_real_v3_route_file_passes_the_rule() {
    let files = v3_route_files();
    assert!(
        files.len() >= 40,
        "the v3 route files were found: {}",
        files.len()
    );
    let failing: Vec<String> = files
        .iter()
        .filter_map(|file| {
            route_file_verdict(&code_of(file)).err().map(|why| {
                let rel = file.strip_prefix(api_dir()).unwrap_or(file);
                format!("{}: {why}", rel.display())
            })
        })
        .collect();
    assert_eq!(
        failing,
        Vec::<String>::new(),
        "v3 route files the rule refuses"
    );
}
