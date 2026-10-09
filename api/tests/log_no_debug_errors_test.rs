//! CI-9 (#656): no `Debug`-formatted error in any log. Design `docs/design/blueprint/customer-identity-consent.md`
//! §2 K10 (after PR 647): a Postgres error's `Debug` prints its `detail` (`Key (…)=(…)`, the clashing value),
//! its `Display` prints only the message. xchange logged errors with `error = ?…` at 14 sites in `api/src`.
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken
//! these tests; add your own in another file.
//!
//! `debug_prints_the_detail_display_does_not` needs `TEST_DATABASE_URL` (a database whose name ends in
//! `_test`), like the other DB tests here; it fails without it, it does not skip.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every line under `api/src` that logs a field named `error` with `?` (Debug), spaces ignored.
fn debug_error_fields() -> Vec<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    let mut hits = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            let squashed: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            if squashed.contains("error=?") {
                hits.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }
    hits
}

#[test]
fn no_log_line_in_api_src_formats_an_error_with_debug() {
    let hits = debug_error_fields();
    assert!(
        hits.is_empty(),
        "log errors with `error = %e` (Display), never `error = ?e` (Debug prints a database error's detail, \
         which carries the clashing value). {} site(s):\n{}",
        hits.len(),
        hits.join("\n")
    );
}

#[test]
fn the_guard_sees_a_debug_error_field_however_it_is_spaced() {
    // The guard itself must not be blind: these spellings are all `?` (Debug) on a field named `error`.
    for line in [
        "tracing::error!(error = ?e, \"x\");",
        "tracing::error!(error=?e, \"x\");",
        "    error = ?error,",
        "tracing::warn!(error =  ?other);",
    ] {
        let squashed: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(squashed.contains("error=?"), "{line}");
    }
    // …and `%` (Display) is not a hit.
    let ok: String = "tracing::error!(error = %e, \"x\");"
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(!ok.contains("error=?"));
}

/// Pins the reason for the rule against a real Postgres unique violation, so the rule is not cargo cult.
#[tokio::test]
async fn debug_prints_the_detail_display_does_not() {
    let url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL (a database whose name ends in _test)");
    assert!(
        url.rsplit('/')
            .next()
            .unwrap_or("")
            .split('?')
            .next()
            .unwrap_or("")
            .ends_with("_test")
    );
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect");
    let mut conn = pool.acquire().await.expect("acquire");
    sqlx::query("CREATE TEMP TABLE ci9_probe (v text UNIQUE)")
        .execute(&mut *conn)
        .await
        .expect("create");
    sqlx::query("INSERT INTO ci9_probe VALUES ('PASSPORT:THA:AA1234567')")
        .execute(&mut *conn)
        .await
        .expect("first");
    let err = sqlx::query("INSERT INTO ci9_probe VALUES ('PASSPORT:THA:AA1234567')")
        .execute(&mut *conn)
        .await
        .expect_err("the second insert violates UNIQUE");
    let app = pos_error::AppError::Database(err);
    let shown = format!("{app}");
    let debugged = format!("{app:?}");
    assert!(
        !shown.contains("AA1234567") && !shown.contains("Key ("),
        "Display must not carry the value: {shown}"
    );
    assert!(
        debugged.contains("AA1234567"),
        "Debug carries the clashing value (this is why the rule exists): {debugged}"
    );
}
