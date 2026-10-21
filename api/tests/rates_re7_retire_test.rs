//! Ticket #604 (RE-7) — the direct rate save is retired and the `rates` table is dropped.
//! Written before the implementation by claude/dell-claude-5 on the lane controller's order
//! (claude/608dff, 2026-10-10). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §8 migration 6 and §9 RE-7:
//! - "`adminSaveRate` is gone (404/refused)";
//! - "`rates` is absent";
//! - "every reader still answers".
//!
//! Done criterion 3 of #604 (the two staged-hash no-ops leave `rate_publication.rs`) is held
//! here as a source scan too, so that it is checked by CI and not only by the reviewer.
//!
//! The source scans read `api/src`, `api/build.rs` and every `api/crates/*/src` from disk. They
//! skip comment lines (`//` after leading spaces), so a sentence about the old table is not a
//! reader.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rates_re4_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::path::{Path, PathBuf};

use actix_web::http::Method;
use rpc_common::{App, Call, Cookies, Desk, MAIN, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// `MAIN` (from the migrations) with one publication of USD, and the owner and one
    /// employee of every staff role in `MAIN`, all signed in.
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        rates_re4_common::publish(&desk.db.pool, MAIN, &[("USD", "32.1", "32.6")]).await;
        Room { desk, app, cookies }
    }

    fn cookie(&self, who: &str) -> String {
        self.cookies
            .get(who)
            .unwrap_or_else(|| panic!("{who} is not signed in"))
            .clone()
    }

    async fn rpc(&self, who: &str, action: &str, payload: Value) -> rpc_common::Answer {
        let call = Call::action(ORIGIN, "/api/rpc", action, payload).cookie(&self.cookie(who));
        self.app.send(call).await
    }

    async fn view(&self, who: &str, path: &str) -> rpc_common::Answer {
        let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
            .without("x-xchange-request")
            .without("content-type")
            .without("origin")
            .cookie(&self.cookie(who));
        call.method = Method::GET;
        self.app.send(call).await
    }

    async fn rates_table_exists(&self) -> bool {
        sqlx::query_scalar::<_, bool>("SELECT to_regclass('rates') IS NOT NULL")
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("ask for the rates table")
    }

    async fn publications(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM rate_publications")
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("count rate_publications")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn save_rate_payload() -> Value {
    json!({ "requestId": "00000000-0000-4000-8000-0000000e7001", "code": "USD",
            "buy": 33.1, "sell": 33.6, "partnerName": "", "partnerCost": null,
            "previousUpdated": "" })
}

// ── migration 6 ──────────────────────────────────────────────────────────────

/// §8 migration 6: "Drops `rates`".
#[tokio::test]
async fn the_rates_table_is_absent_after_every_migration() {
    let room = Room::open().await;
    assert!(
        !room.rates_table_exists().await,
        "after api/migrations/ the table `rates` does not exist (§8 migration 6)"
    );
    room.finish().await;
}

// ── the direct save ──────────────────────────────────────────────────────────

/// §9 RE-7: "`adminSaveRate` is gone (404/refused)". The compat route answers it exactly as it
/// answers a name v2 does not know, and nothing is written: no publication, no `rates` row.
#[tokio::test]
async fn admin_save_rate_is_answered_as_a_name_v2_does_not_know() {
    let room = Room::open().await;
    let before = room.publications().await;

    let unknown = room
        .rpc("OWNER", "re7NoSuchAction", save_rate_payload())
        .await;
    assert_ne!(
        unknown.status, 200,
        "a name v2 does not know is refused: {}",
        unknown.text
    );
    let save = room
        .rpc("OWNER", "adminSaveRate", save_rate_payload())
        .await;
    assert_eq!(
        (save.status, save.text.as_str()),
        (unknown.status, unknown.text.as_str()),
        "adminSaveRate is answered as an unknown name (§9 RE-7)"
    );

    assert_eq!(
        room.publications().await,
        before,
        "a refused adminSaveRate publishes nothing"
    );
    if room.rates_table_exists().await {
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM rates")
            .fetch_one(&room.desk.db.pool)
            .await
            .expect("count rates");
        assert_eq!(rows, 0, "adminSaveRate wrote a row to `rates`");
    }
    room.finish().await;
}

// ── every reader still answers ───────────────────────────────────────────────

/// §9 RE-7: "every reader still answers". With no `rates` table present (dropped here if a
/// migration left it), the v2 snapshot, the v2 desk config and the v3 views that read rates
/// answer 200. Each reader is called once before the drop as well, so a refusal that has nothing
/// to do with `rates` shows up as such.
#[tokio::test]
async fn every_reader_still_answers_with_no_rates_table() {
    let room = Room::open().await;
    let views = [
        format!("/api/v3/views/desk_config?branch={MAIN}"),
        format!("/api/v3/views/admin_cash?branch={MAIN}"),
    ];

    let mut before = Vec::new();
    for action in ["adminSnapshot", "adminDeskConfig"] {
        before.push((
            action.to_string(),
            room.rpc("OWNER", action, json!({})).await.status,
        ));
    }
    for path in &views {
        before.push((path.clone(), room.view("OWNER", path).await.status));
    }
    for (what, status) in &before {
        assert_eq!(*status, 200, "{what} answers 200 before `rates` is dropped");
    }

    sqlx::query("DROP TABLE IF EXISTS rates CASCADE")
        .execute(&room.desk.db.pool)
        .await
        .expect("drop rates");

    for action in ["adminSnapshot", "adminDeskConfig"] {
        let a = room.rpc("OWNER", action, json!({})).await;
        assert_eq!(a.status, 200, "{action} with no `rates` table: {}", a.text);
        let body = a.json();
        assert!(
            body.is_object(),
            "{action} with no `rates` table answers an object: {body}"
        );
    }
    for path in &views {
        let a = room.view("OWNER", path).await;
        assert_eq!(a.status, 200, "{path} with no `rates` table: {}", a.text);
    }
    room.finish().await;
}

// ── source scans ─────────────────────────────────────────────────────────────

fn api_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `api/src`, `api/crates/*/src`, and `api/build.rs`.
fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries =
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push(path);
            }
        }
    }
    let api = api_dir();
    let mut files = vec![api.join("build.rs")];
    walk(&api.join("src"), &mut files);
    let crates = std::fs::read_dir(api.join("crates")).expect("read api/crates");
    for krate in crates {
        let src = krate.expect("a crate").path().join("src");
        if src.is_dir() {
            walk(&src, &mut files);
        }
    }
    assert!(
        files.len() > 100,
        "the scan found the sources: {}",
        files.len()
    );
    files
}

/// `(file:line, line)` of every non-comment source line that `hit` matches.
fn scan(hit: impl Fn(&str) -> bool) -> Vec<String> {
    let api = api_dir();
    let mut found = Vec::new();
    for file in source_files() {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if hit(line) {
                let rel = file
                    .strip_prefix(&api)
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                found.push(format!("api/{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    found
}

/// Whether `line` names the table `rates` after an SQL keyword that reads or writes a table
/// (`FROM rates`, `JOIN rates`, `INTO rates`, `UPDATE rates`, `TABLE rates`), in any case.
/// `published_rates` and `rates_x` are other names.
fn names_the_rates_table(line: &str) -> bool {
    let words: Vec<String> = line
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    words.windows(2).any(|pair| {
        matches!(
            pair[0].as_str(),
            "from" | "join" | "into" | "update" | "table"
        ) && pair[1] == "rates"
    })
}

#[test]
fn the_rates_matcher_tells_the_table_from_other_names() {
    assert!(names_the_rates_table(
        "\"SELECT buy FROM rates WHERE branch_id = $1\","
    ));
    assert!(names_the_rates_table(
        "INSERT INTO rates (branch_id) VALUES ($1)"
    ));
    assert!(names_the_rates_table("x JOIN rates r ON r.code = c.code"));
    assert!(names_the_rates_table(
        "\"SELECT DISTINCT trim(code) FROM rates\""
    ));
    assert!(!names_the_rates_table("SELECT * FROM published_rates"));
    assert!(!names_the_rates_table("FROM rate_publications p"));
    assert!(!names_the_rates_table("let rates = from(rates_x);"));
}

/// §9 RE-7 "every reader has moved", §8 migration 6: no source line reads or writes `rates`.
#[test]
fn no_source_reads_or_writes_the_rates_table() {
    let found = scan(names_the_rates_table);
    assert!(
        found.is_empty(),
        "these lines still read or write the dropped `rates` table:\n{}",
        found.join("\n")
    );
}

/// §9 RE-7: `RateService::save_rate` removed and `adminSaveRate` no longer dispatched.
#[test]
fn the_direct_save_is_gone_from_the_source() {
    let found = scan(|line| line.contains("fn save_rate") || line.contains(".save_rate("));
    assert!(
        found.is_empty(),
        "RateService::save_rate and its callers remain:\n{}",
        found.join("\n")
    );
    let rpc = std::fs::read_to_string(api_dir().join("src/application/compat/rpc.rs"))
        .expect("read api/src/application/compat/rpc.rs");
    let dispatched: Vec<&str> = rpc
        .lines()
        .filter(|l| !l.trim_start().starts_with("//") && l.contains("\"adminSaveRate\""))
        .collect();
    assert!(
        dispatched.is_empty(),
        "compat rpc.rs still names adminSaveRate:\n{}",
        dispatched.join("\n")
    );
}

/// §9 RE-7: `Quote::compute` removed (its callers price on the published line).
#[test]
fn quote_compute_is_gone_from_the_source() {
    let found = scan(|line| line.contains("Quote::compute"));
    let quote = std::fs::read_to_string(api_dir().join("crates/xc-kernel/src/money/quote.rs"))
        .expect("read api/crates/xc-kernel/src/money/quote.rs");
    let defined: Vec<&str> = quote
        .lines()
        .filter(|l| !l.trim_start().starts_with("//") && l.contains("fn compute("))
        .collect();
    assert!(
        found.is_empty() && defined.is_empty(),
        "Quote::compute remains:\n{}\n{}",
        found.join("\n"),
        defined.join("\n")
    );
}

/// #604 done criterion 3: `git grep -n "stage_evaluation_hash\|unstage_evaluation_hash\|
/// STAGED_HASHES" origin/main -- api` prints nothing.
#[test]
fn the_staged_evaluation_hash_no_ops_are_gone() {
    let found =
        scan(|line| line.contains("stage_evaluation_hash") || line.contains("STAGED_HASHES"));
    assert!(
        found.is_empty(),
        "the staged-hash no-ops of rate_publication.rs remain:\n{}",
        found.join("\n")
    );
}
