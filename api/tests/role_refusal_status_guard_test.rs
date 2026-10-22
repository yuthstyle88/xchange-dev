//! Guard (#4586): handler code must not hand-build `roleNotAllowed` / `roleNotAllowedHere`
//! refusals with HTTP 403 (`PolicyError::status` contract is 400, #4512).

use std::fs;
use std::path::{Path, PathBuf};

// Non-empty entries require the lane controller's ruling cited in a comment.
const ALLOWLIST: &[&str] = &[];

fn strip_comments_and_cfg_test(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
        } else if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                if out[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            if i + 1 < b.len() {
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
            }
        } else {
            i += 1;
        }
    }

    let stripped = String::from_utf8(out).expect("utf-8");
    let mut out = stripped.as_bytes().to_vec();
    let mut from = 0;
    while let Some(off) = stripped[from..].find("#[cfg(test)]") {
        let start = from + off;
        let mut idx = start + "#[cfg(test)]".len();
        while idx < out.len() && out[idx] != b'{' && out[idx] != b';' {
            idx += 1;
        }
        if idx < out.len() && out[idx] == b'{' {
            let mut depth = 1usize;
            idx += 1;
            while idx < out.len() && depth > 0 {
                if out[idx] == b'"' {
                    idx += 1;
                    while idx < out.len() && out[idx] != b'"' {
                        if out[idx] == b'\\' {
                            idx += 1;
                        }
                        idx += 1;
                    }
                } else if out[idx] == b'{' {
                    depth += 1;
                } else if out[idx] == b'}' {
                    depth -= 1;
                }
                idx += 1;
            }
            for ch in &mut out[start..idx] {
                if *ch != b'\n' {
                    *ch = b' ';
                }
            }
        }
        from = idx.max(start + 1);
    }

    String::from_utf8(out).expect("utf-8")
}

fn call_args(code: &str, start: usize) -> Option<&str> {
    let b = code.as_bytes();
    let mut depth = 1usize;
    let mut i = start;
    while i < b.len() {
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
        } else if b[i] == b'(' {
            depth += 1;
        } else if b[i] == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(&code[start..i]);
            }
        }
        i += 1;
    }
    None
}

fn is_role_refusal_code(arg: &str) -> bool {
    matches!(arg.trim(), "\"roleNotAllowed\"" | "\"roleNotAllowedHere\"")
}

fn scan_source(rel: &str, src: &str) -> Vec<String> {
    let code = strip_comments_and_cfg_test(src);
    let mut hits = Vec::new();

    let v3_needle = "V3Error::new(";
    let mut from = 0;
    while let Some(off) = code[from..].find(v3_needle) {
        let at = from + off;
        let open = at + v3_needle.len();
        from = open;
        if let Some(args) = call_args(&code, open)
            && args.split(',').next().is_some_and(is_role_refusal_code)
            && args.contains("StatusCode::FORBIDDEN")
        {
            let line = code[..at].bytes().filter(|&b| b == b'\n').count() + 1;
            hits.push(format!("{rel}:{line}"));
        }
    }

    let refuse_needle = "refuse(";
    let mut from = 0;
    while let Some(off) = code[from..].find(refuse_needle) {
        let at = from + off;
        let open = at + refuse_needle.len();
        from = open;
        if at > 0 {
            let prev = code.as_bytes()[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                continue;
            }
        }
        if let Some(args) = call_args(&code, open) {
            let mut parts = args.splitn(3, ',');
            let status = parts.next().map(str::trim).unwrap_or("");
            let err_code = parts.next().unwrap_or("");
            if status == "403" && is_role_refusal_code(err_code) {
                let line = code[..at].bytes().filter(|&b| b == b'\n').count() + 1;
                hits.push(format!("{rel}:{line}"));
            }
        }
    }

    hits
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            walk_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn role_refusal_status_guard() {
    let api_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = api_dir.parent().unwrap_or(api_dir);

    let mut roots = vec![api_dir.join("src")];
    let mut crates: Vec<PathBuf> = fs::read_dir(api_dir.join("crates"))
        .expect("read api/crates")
        .map(|e| e.expect("crate entry").path())
        .filter(|p| p.is_dir())
        .collect();
    crates.sort();
    for crate_dir in crates {
        let src = crate_dir.join("src");
        if src.is_dir() {
            roots.push(src);
        }
    }

    let mut files = Vec::new();
    for root in &roots {
        walk_rs(root, &mut files);
    }
    assert!(!files.is_empty(), "found no .rs files to scan");

    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(repo_root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOWLIST.contains(&rel.as_str()) {
            continue;
        }
        let src = fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {rel}: {e}"));
        for hit in scan_source(&rel, &src) {
            if !ALLOWLIST.contains(&hit.as_str()) {
                violations.push(hit);
            }
        }
    }

    assert!(
        violations.is_empty(),
        "hand-built 403 role refusal(s) found (contract is 400 via PolicyError::status):\n{}",
        violations.join("\n")
    );
}

#[test]
fn role_refusal_status_guard_catches_both_patterns_and_skips_cfg_test() {
    let sample = r#"
fn bad_v3() -> V3Error {
    V3Error::new(
        "roleNotAllowed",
        None,
        "denied",
        StatusCode::FORBIDDEN,
    )
}
fn bad_refuse() -> HttpResponse {
    refuse(403, "roleNotAllowedHere", "denied")
}
fn ok_400() -> HttpResponse {
    refuse(400, "roleNotAllowed", "denied")
}
#[cfg(test)]
mod tests {
    fn ignored() {
        let _ = V3Error::new("roleNotAllowedHere", None, "x", StatusCode::FORBIDDEN);
        let _ = refuse(403, "roleNotAllowed", "x");
    }
}
"#;
    assert_eq!(
        scan_source("api/src/sample.rs", sample),
        vec!["api/src/sample.rs:3", "api/src/sample.rs:11"]
    );
}
