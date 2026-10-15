// Written before the implementation by the ticket author claude/d7aa64. Do not edit, delete or weaken.

//! Guard: no presentation code turns a whole error into a non-500 refusal by its `Display` text.
//!
//! Two shapes are flagged (both build `V3Error::new(<code>, <field>, <id>.to_string(), <status>)`
//! where `<id>` is the very value being matched / mapped, and `<status>` is not 500):
//!
//! - ARM:   `<id> => { V3Error::new("…", …, <id>.to_string(), StatusCode::X)` — a catch-all match arm.
//! - AWAIT: `.await.map_err(|<id>| { V3Error::new("…", …, <id>.to_string(), StatusCode::X)` — a whole
//!   service error mapped after an await.
//!
//! Such a site sends store text (sqlx, Postgres) to the client as a 4xx and hides the real kind
//! of failure. The domain enums map through `From<X> for V3Error` instead.
//!
//! The api crate has no `regex` dependency (and the manifest is shared), so the matcher below is
//! written by hand; it follows the two patterns token for token.

use std::fs;
use std::path::{Path, PathBuf};

const CEILING: usize = 0;

#[derive(Debug, PartialEq, Eq)]
struct Hit {
    shape: &'static str,
    id: String,
    status: String,
    /// Byte offset of `V3Error::new(` in the scanned text (`//` lines blanked, so lines keep their numbers).
    at: usize,
}

fn is_ws(b: u8) -> bool {
    b.is_ascii_whitespace()
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && is_ws(s[i]) {
        i += 1;
    }
    i
}

fn skip_ws_back(s: &[u8], mut i: usize) -> usize {
    while i > 0 && is_ws(s[i - 1]) {
        i -= 1;
    }
    i
}

fn eat(s: &[u8], i: usize, lit: &str) -> Option<usize> {
    s[i..].starts_with(lit.as_bytes()).then_some(i + lit.len())
}

fn eat_back(s: &[u8], i: usize, lit: &str) -> Option<usize> {
    (i >= lit.len() && &s[i - lit.len()..i] == lit.as_bytes()).then(|| i - lit.len())
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `[a-z_][a-z0-9_]*` forward.
fn ident(s: &[u8], i: usize) -> Option<(usize, String)> {
    let first = *s.get(i)?;
    if !(first.is_ascii_lowercase() || first == b'_') {
        return None;
    }
    let mut j = i + 1;
    while j < s.len() && (s[j].is_ascii_lowercase() || s[j].is_ascii_digit() || s[j] == b'_') {
        j += 1;
    }
    Some((j, String::from_utf8_lossy(&s[i..j]).into_owned()))
}

/// `\b[a-z_][a-z0-9_]*` ending at `end` (exclusive), read backwards.
fn ident_back(s: &[u8], end: usize) -> Option<(usize, String)> {
    let mut start = end;
    while start > 0 && is_word(s[start - 1]) {
        start -= 1;
    }
    if start == end {
        return None;
    }
    let (stop, id) = ident(s, start)?;
    (stop == end).then_some((start, id))
}

/// From just after `V3Error::new(`: `\s*"[^"]*"\s*,\s*[^,]+,\s*<id2>\.to_string\(\)\s*,\s*(?:actix_web::http::)?StatusCode::<st>`.
fn args(s: &[u8], i: usize) -> Option<(String, String)> {
    let mut i = skip_ws(s, i);
    i = eat(s, i, "\"")?;
    while i < s.len() && s[i] != b'"' {
        i += 1;
    }
    i = eat(s, i, "\"")?;
    i = skip_ws(s, i);
    i = eat(s, i, ",")?;
    i = skip_ws(s, i);
    let field_start = i;
    while i < s.len() && s[i] != b',' {
        i += 1;
    }
    if i == field_start {
        return None;
    }
    i = eat(s, i, ",")?;
    i = skip_ws(s, i);
    let (j, id2) = ident(s, i)?;
    i = eat(s, j, ".to_string()")?;
    i = skip_ws(s, i);
    i = eat(s, i, ",")?;
    i = skip_ws(s, i);
    i = eat(s, i, "actix_web::http::").unwrap_or(i);
    i = eat(s, i, "StatusCode::")?;
    let st_start = i;
    while i < s.len() && (s[i].is_ascii_uppercase() || s[i] == b'_') {
        i += 1;
    }
    if i == st_start {
        return None;
    }
    Some((id2, String::from_utf8_lossy(&s[st_start..i]).into_owned()))
}

/// Before `V3Error::new(` at `at`: `\s*\{?\s*` then the shape's own prefix.
fn before_brace(s: &[u8], at: usize) -> usize {
    let i = skip_ws_back(s, at);
    let i = eat_back(s, i, "{").unwrap_or(i);
    skip_ws_back(s, i)
}

/// ARM: `\b<id>\s*=>` ending at `i`.
fn arm_id(s: &[u8], i: usize) -> Option<String> {
    let i = eat_back(s, i, "=>")?;
    let i = skip_ws_back(s, i);
    ident_back(s, i).map(|(_, id)| id)
}

/// AWAIT: `\.await\s*\.map_err\(\s*\|<id>\|` ending at `i`.
fn await_id(s: &[u8], i: usize) -> Option<String> {
    let i = eat_back(s, i, "|")?;
    let (start, id) = ident_back(s, i)?;
    let i = eat_back(s, start, "|")?;
    let i = skip_ws_back(s, i);
    let i = eat_back(s, i, ".map_err(")?;
    let i = skip_ws_back(s, i);
    eat_back(s, i, ".await")?;
    Some(id)
}

fn scan(text: &str) -> Vec<Hit> {
    let kept: String = text
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("//") {
                ""
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let s = kept.as_bytes();
    let needle = "V3Error::new(";
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(off) = kept[from..].find(needle) {
        let at = from + off;
        from = at + needle.len();
        let Some((id2, status)) = args(s, at + needle.len()) else {
            continue;
        };
        if status == "INTERNAL_SERVER_ERROR" {
            continue;
        }
        let i = before_brace(s, at);
        if let Some(id) = arm_id(s, i).filter(|id| *id == id2) {
            hits.push(Hit {
                shape: "ARM",
                id,
                status,
                at,
            });
        } else if let Some(id) = await_id(s, i).filter(|id| *id == id2) {
            hits.push(Hit {
                shape: "AWAIT",
                id,
                status,
                at,
            });
        }
    }
    hits
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read a presentation directory") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_presentation_code_refuses_with_an_errors_display_text() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presentation");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "found no .rs file under {root:?}");

    let mut found = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read a source file");
        let kept: String = text
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("//") {
                    ""
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        for hit in scan(&text) {
            let line = kept[..hit.at].matches('\n').count() + 1;
            let rel = file
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap_or(file);
            found.push(format!(
                "{}: {} `{}` -> StatusCode::{} (V3Error::new at line {line})",
                rel.display(),
                hit.shape,
                hit.id,
                hit.status
            ));
        }
    }
    assert!(
        found.len() == CEILING,
        "{} catch-all site(s) answer a non-500 refusal with the error's Display text \
         (ceiling {CEILING}); map the domain error through `From<X> for V3Error` instead:\n{}",
        found.len(),
        found.join("\n")
    );
}

// ------------------------------------------------------------ fixture self-tests

#[test]
fn fixture_arm_catch_all_is_flagged() {
    let src = r#"
        .map_err(|err| match err {
            SlotError::NotFound => V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND),
            other => V3Error::new(
                "badRequest",
                None,
                other.to_string(),
                StatusCode::BAD_REQUEST,
            ),
        })?;
    "#;
    let hits = scan(src);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].shape, "ARM");
    assert_eq!(hits[0].id, "other");
    assert_eq!(hits[0].status, "BAD_REQUEST");
}

#[test]
fn fixture_arm_with_brace_and_full_path_is_flagged() {
    let src = r#"e => { V3Error::new("badRequest", Some("x"), e.to_string(), actix_web::http::StatusCode::CONFLICT) }"#;
    let hits = scan(src);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].status, "CONFLICT");
}

#[test]
fn fixture_await_map_err_is_flagged() {
    let src = r#"
        services.slot_service.remove_closure_in(tx, &branch, input.date)
            .await
            .map_err(|e| {
                V3Error::new(
                    "badRequest",
                    None,
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;
    "#;
    let hits = scan(src);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].shape, "AWAIT");
    assert_eq!(hits[0].id, "e");
}

#[test]
fn fixture_500_is_not_flagged() {
    let src = r#"
        BranchTx::lock_also(tx, &branch).await.map_err(|e| {
            V3Error::new("serverError", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR)
        })?;
        match x { other => V3Error::new("database", None, other.to_string(), StatusCode::INTERNAL_SERVER_ERROR) }
    "#;
    assert_eq!(scan(src), Vec::new());
}

#[test]
fn fixture_parse_closure_without_await_is_not_flagged() {
    let src = r#"
        let n = input.parse::<u32>().map_err(|e| V3Error::new("validationFailed", Some("x"), e.to_string(), StatusCode::BAD_REQUEST))?;
    "#;
    assert_eq!(scan(src), Vec::new());
}

#[test]
fn fixture_different_ident_is_not_flagged() {
    let src =
        r#"other => V3Error::new("badRequest", None, inner.to_string(), StatusCode::BAD_REQUEST)"#;
    assert_eq!(scan(src), Vec::new());
}

#[test]
fn fixture_commented_out_line_is_not_flagged() {
    let src = "// other => V3Error::new(\"badRequest\", None, other.to_string(), StatusCode::BAD_REQUEST)";
    assert_eq!(scan(src), Vec::new());
}
