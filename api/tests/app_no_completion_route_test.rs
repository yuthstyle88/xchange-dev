//! Ticket #995 (APP-5) — design P14 (`docs/design/blueprint/customer-app.md`): the customer never
//! moves a booking, ticket or transaction to a done state. No route under `/api/app` names
//! complete, collect, call, check-in, advance, or a transaction state (`COMPLETED`, `REVERSED`:
//! `api/migrations/20261105100000_broker_exchange_feed.sql:10`).
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! The route table is the routes registered under `api/src/presentation/http/` (`http/mod.rs`,
//! "the one route table"), whether a module's own `configure` or a v3 command's `const ROUTE`.
//! A route is written as one full string literal there, so this test reads every string literal
//! in those files that is `/api/app` or starts with `/api/app/`. A literal that only starts with
//! `/api/app` and goes on (`/api/approvals`, `/api/app-settings`) is another route. Because only
//! full literals are read, no file under `http/` may build an `/api/app` route from parts
//! (`web::scope("/api/app")`, `format!`, `concat!`): the second test refuses those.
//!
//! No database.

use std::path::{Path, PathBuf};

/// Lower-cased stems a customer route must not name, compared with `-` and `_` removed.
const FORBIDDEN: &[&str] = &[
    "complete", "collect", "call", "checkin", "advance", "reverse",
];

/// Every `/api/app` route literal in `source`.
fn app_routes(source: &str) -> Vec<String> {
    let mut routes = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("\"/api/app") {
        let after = &rest[at + 1..];
        let end = after.find('"').unwrap_or(after.len());
        let literal = &after[..end];
        if literal == "/api/app" || literal.starts_with("/api/app/") {
            routes.push(literal.to_owned());
        }
        rest = &after[end..];
    }
    routes
}

/// The forbidden stems `route` names.
fn named_stems(route: &str) -> Vec<&'static str> {
    let flat = route.to_lowercase().replace(['-', '_'], "");
    FORBIDDEN
        .iter()
        .copied()
        .filter(|stem| flat.contains(stem))
        .collect()
}

/// Ways of building an `/api/app` route from parts, which `app_routes` would not see.
fn built_from_parts(source: &str) -> bool {
    let flat: String = source.split_whitespace().collect();
    flat.contains("scope(\"/api/app\"")
        || flat.contains("scope(\"/api/app/")
        || (flat.contains("\"/api/app/{}") || flat.contains("\"/api/app{}"))
        || flat.contains("concat!(\"/api/app")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn http_sources() -> Vec<(PathBuf, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presentation/http");
    let mut files = Vec::new();
    rust_files(&dir, &mut files);
    assert!(
        files.len() > 10,
        "found the route modules under {}",
        dir.display()
    );
    files
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("read a route module");
            (p, text)
        })
        .collect()
}

#[test]
fn no_customer_route_names_a_done_state() {
    let mut offending = Vec::new();
    for (path, source) in http_sources() {
        for route in app_routes(&source) {
            let stems = named_stems(&route);
            if !stems.is_empty() {
                offending.push(format!("{}: {route} names {stems:?}", path.display()));
            }
        }
    }
    assert!(
        offending.is_empty(),
        "P14: the customer never completes anything; the teller does:\n{}",
        offending.join("\n")
    );
}

#[test]
fn no_customer_route_is_built_from_parts() {
    let offending: Vec<String> = http_sources()
        .into_iter()
        .filter(|(_, source)| built_from_parts(source))
        .map(|(path, _)| path.display().to_string())
        .collect();
    assert!(
        offending.is_empty(),
        "write each /api/app route as one full literal, so the P14 guard can read it: {offending:?}"
    );
}

/// The guard's own reading, on sources made up for it: what it must catch and what it must leave.
#[test]
fn the_guard_reads_what_it_should() {
    let source = r#"
        const ROUTE: &'static str = "/api/app/planner-drafts/{id}";
        cfg.route("/api/app/bookings/{id}/check-in", web::post().to(h));
        cfg.route("/api/app/tickets/{id}/call", web::post().to(h));
        cfg.route("/api/app/transactions/{id}/COMPLETED", web::post().to(h));
        cfg.route("/api/app/orders/{id}/collect_cash", web::post().to(h));
        cfg.route("/api/approvals/{id}/decide", web::post().to(h));
        cfg.route("/api/app-settings", web::put().to(h));
        cfg.route("/api/staff/bookings/{id}/complete", web::post().to(h));
    "#;
    let routes = app_routes(source);
    assert_eq!(
        routes,
        [
            "/api/app/planner-drafts/{id}",
            "/api/app/bookings/{id}/check-in",
            "/api/app/tickets/{id}/call",
            "/api/app/transactions/{id}/COMPLETED",
            "/api/app/orders/{id}/collect_cash",
        ],
        "only /api/app routes; /api/approvals, /api/app-settings and staff routes are not the app's"
    );
    let named: Vec<Vec<&str>> = routes.iter().map(|r| named_stems(r)).collect();
    assert_eq!(
        named,
        [
            vec![],
            vec!["checkin"],
            vec!["call"],
            vec!["complete"],
            vec!["collect"],
        ]
    );

    assert!(built_from_parts(
        r#"web::scope( "/api/app" ).route("/x", web::get().to(h))"#
    ));
    assert!(built_from_parts(r#"format!("/api/app/{}/done", id)"#));
    assert!(built_from_parts(r#"concat!("/api/app", "/x")"#));
    assert!(!built_from_parts(
        r#"cfg.route("/api/app/planner-drafts", web::post().to(h));"#
    ));
}
