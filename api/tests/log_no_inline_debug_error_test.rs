//! CI-9 / K10 (#692): no inline `Debug`-formatted error in logs or format strings.
//! Design `docs/design/blueprint/customer-identity-consent.md` §2 K10:
//! "no `Debug`-formatted error in any log". A Postgres error's `Debug` prints its `detail`
//! (the clashing value), and its `Display` does not.
//!
//! While `api/tests/log_no_debug_errors_test.rs` (#656) guards against `error = ?e`,
//! this test guards against inline format strings like `"{e:?}"`, `"{err:#?}"`,
//! or `"{db_error:?}"` in `api/src` and `api/crates/*/src`.
//!
//! Note on edge cases: Positional `{:?}` with an error argument is not caught.
//! That is intentional: the matcher has no type information. Multi-line macro
//! calls are scanned line-by-line and caught on their placeholder line.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn scan_roots() -> Vec<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut roots = vec![manifest_dir.join("src")];
    let crates_dir = manifest_dir.join("crates");
    if let Ok(entries) = std::fs::read_dir(&crates_dir) {
        let mut crate_paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .map(|p| p.join("src"))
            .filter(|p| p.is_dir())
            .collect();
        crate_paths.sort();
        roots.extend(crate_paths);
    }
    roots
}

fn is_error_name(name: &str) -> bool {
    let name = name.trim();
    if name == "e" || name == "err" || name == "error" {
        return true;
    }
    if let Some(prefix) = name.strip_suffix("_err") {
        return !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_lowercase() || c == '_');
    }
    if let Some(prefix) = name.strip_suffix("_error") {
        return !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_lowercase() || c == '_');
    }
    false
}

fn has_inline_debug_error(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                // Escaped {{ in format string
                i += 2;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'}' && bytes[end] != b'{' {
                end += 1;
            }
            if end < bytes.len() && bytes[end] == b'}' {
                let inner = &text[start..end];
                if let Some((name, spec)) = inner.split_once(':') {
                    let spec = spec.trim();
                    if (spec == "?" || spec == "#?") && is_error_name(name) {
                        return true;
                    }
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    false
}

fn inline_debug_error_sites() -> Vec<String> {
    let mut files = Vec::new();
    for root in scan_roots() {
        rust_files(&root, &mut files);
    }
    files.sort();
    files.dedup();

    let mut hits = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            if has_inline_debug_error(line) {
                hits.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }
    hits
}

#[test]
fn no_source_file_formats_an_inline_debug_error() {
    let hits = inline_debug_error_sites();
    assert!(
        hits.is_empty(),
        "log errors with Display (`{{e}}` / `error = %e`), never Debug. {} site(s):\n{}",
        hits.len(),
        hits.join("\n")
    );
}

#[test]
fn the_guard_sees_inline_debug_errors() {
    for hit in ["x: {e:?}", "{err:#?}", "{db_error:?}"] {
        assert!(has_inline_debug_error(hit), "expected hit for {hit}");
    }
    for miss in ["{e}", "{other:?}", "{:?}"] {
        assert!(!has_inline_debug_error(miss), "expected miss for {miss}");
    }
}
