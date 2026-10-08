//! Guard test ensuring no DTO uses `#[ts(export)]` (ticket #504).
//!
//! `#[ts(export)]` makes `cargo test` write bindings to `api/bindings/*.ts`,
//! which dirties the git tree and causes `build_identity_test` to fail.
//! Only `#[ts(export_to = "...")]` is permitted.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct Violation {
    path: PathBuf,
    line_number: usize,
    line_content: String,
}

/// Strips line comments `//...` from the line, taking care not to strip `//`
/// inside string literals.
fn strip_line_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut escaped = false;
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if escaped {
            escaped = false;
            continue;
        }
        if bytes[i] == b'\\' {
            escaped = true;
            continue;
        }
        if bytes[i] == b'"' {
            in_quote = !in_quote;
        } else if !in_quote && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            return &line[..i];
        }
    }
    line
}

/// Finds the index of the matching closing parenthesis `)` outside quotes.
fn find_matching_paren(s: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut escaped = false;
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '"' {
            in_quote = !in_quote;
        } else if !in_quote {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
        }
    }
    None
}

/// Checks if an argument inside `#[ts(...)]` is a forbidden `export` flag.
fn is_forbidden_export_item(item: &str) -> bool {
    let item = item.trim();
    if item == "export" {
        return true;
    }
    if item.starts_with("export") && !item.starts_with("export_to") {
        let rest = &item["export".len()..];
        if rest.starts_with(|c: char| {
            c.is_whitespace() || c == '=' || c == '(' || c == ',' || c == ')'
        }) {
            return true;
        }
    }
    false
}

/// Checks if comma-separated items inside `#[ts(...)]` contain forbidden `export`.
fn contains_forbidden_export(inside: &str) -> bool {
    let mut in_quote = false;
    let mut escaped = false;
    let mut start = 0;
    let bytes = inside.as_bytes();
    for i in 0..bytes.len() {
        if escaped {
            escaped = false;
            continue;
        }
        if bytes[i] == b'\\' {
            escaped = true;
            continue;
        }
        if bytes[i] == b'"' {
            in_quote = !in_quote;
        } else if !in_quote && bytes[i] == b',' {
            let item = &inside[start..i];
            if is_forbidden_export_item(item) {
                return true;
            }
            start = i + 1;
        }
    }
    if start < inside.len() {
        let item = &inside[start..];
        if is_forbidden_export_item(item) {
            return true;
        }
    }
    false
}

/// Checks whether `code` (with comments stripped) starts a `ts(...)` attribute
/// that is preceded by `#[` or `#![`.
fn is_ts_attribute(prefix: &str) -> bool {
    let trimmed = prefix.trim_end();
    if let Some(open_bracket) = trimmed.rfind('[') {
        let after_open = &trimmed[open_bracket + 1..];
        if !after_open.contains(']') {
            let before_open = trimmed[..open_bracket].trim_end();
            return before_open.ends_with('#') || before_open.ends_with("#!");
        }
    }
    false
}

/// Checks a single line of Rust code for forbidden `ts(export...)`.
/// Also updates `in_ts_attr` state in case of multi-line attributes.
fn check_line_for_ts_export(line: &str, in_ts_attr: &mut bool) -> bool {
    let code = strip_line_comment(line);
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return false;
    }

    if *in_ts_attr {
        if let Some(close_idx) = find_matching_paren(code) {
            let inside = &code[..close_idx];
            *in_ts_attr = false;
            if contains_forbidden_export(inside) {
                return true;
            }
            return check_line_for_ts_export(&code[close_idx + 1..], in_ts_attr);
        } else {
            return contains_forbidden_export(code);
        }
    }

    let mut rem = code;
    while let Some(pos) = rem.find("ts(") {
        let prefix = &rem[..pos];
        if is_ts_attribute(prefix) {
            let after_ts = &rem[pos + 3..];
            if let Some(close_idx) = find_matching_paren(after_ts) {
                let inside = &after_ts[..close_idx];
                if contains_forbidden_export(inside) {
                    return true;
                }
                rem = &after_ts[close_idx + 1..];
                continue;
            } else {
                *in_ts_attr = true;
                if contains_forbidden_export(after_ts) {
                    return true;
                }
                return false;
            }
        }
        rem = &rem[pos + 3..];
    }
    false
}

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == "target" || name_str.starts_with('.') {
                continue;
            }
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn no_ts_export_in_api_src_and_crates() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap_or(manifest_dir);

    let mut files = Vec::new();
    collect_rs_files(&manifest_dir.join("src"), &mut files);
    collect_rs_files(&manifest_dir.join("crates"), &mut files);
    files.sort();

    assert!(
        !files.is_empty(),
        "Expected to find *.rs files under api/src and api/crates"
    );

    let mut violations = Vec::new();

    for file_path in &files {
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                panic!("Failed to read {}: {}", file_path.display(), e);
            }
        };

        let mut in_ts_attr = false;
        for (idx, line) in content.lines().enumerate() {
            let line_number = idx + 1;
            if check_line_for_ts_export(line, &mut in_ts_attr) {
                let display_path = file_path
                    .strip_prefix(repo_root)
                    .unwrap_or(file_path)
                    .to_path_buf();
                violations.push(Violation {
                    path: display_path,
                    line_number,
                    line_content: line.trim().to_string(),
                });
            }
        }
    }

    if !violations.is_empty() {
        let mut msg =
            String::from("Found forbidden #[ts(export...)] attribute in the following file(s):\n");
        for v in &violations {
            msg.push_str(&format!(
                "  {}:{}: {}\n",
                v.path.display(),
                v.line_number,
                v.line_content
            ));
        }
        msg.push_str(
            "DTOs must use #[ts(export_to = \"...\")] only, never #[ts(export)] or #[ts(export, ...)].\n",
        );
        panic!("{msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_to_alone_passes() {
        let mut in_ts = false;
        assert!(!check_line_for_ts_export(
            r#"#[ts(export_to = "common.ts")]"#,
            &mut in_ts
        ));
        assert!(!check_line_for_ts_export(
            r#"#[ts(export_to="common.ts")]"#,
            &mut in_ts
        ));
        assert!(!check_line_for_ts_export(
            r#"#[ts(rename_all = "camelCase", export_to = "common.ts")]"#,
            &mut in_ts
        ));
    }

    #[test]
    fn test_commented_out_line_is_ignored() {
        let mut in_ts = false;
        assert!(!check_line_for_ts_export(
            r#"// #[ts(export, export_to = "x.ts")]"#,
            &mut in_ts
        ));
        assert!(!check_line_for_ts_export(
            r#"   // #[ts(export)]"#,
            &mut in_ts
        ));
        assert!(!check_line_for_ts_export(
            r#"/// #[ts(export)]"#,
            &mut in_ts
        ));
    }

    #[test]
    fn test_export_first_fails() {
        let mut in_ts = false;
        assert!(check_line_for_ts_export(
            r#"#[ts(export, export_to = "x.ts")]"#,
            &mut in_ts
        ));
    }

    #[test]
    fn test_export_alone_fails() {
        let mut in_ts = false;
        assert!(check_line_for_ts_export(r#"#[ts(export)]"#, &mut in_ts));
        assert!(check_line_for_ts_export(r#"#[ts( export )]"#, &mut in_ts));
    }

    #[test]
    fn test_export_last_fails() {
        let mut in_ts = false;
        assert!(check_line_for_ts_export(
            r#"#[ts(rename_all = "camelCase", export)]"#,
            &mut in_ts
        ));
        assert!(check_line_for_ts_export(
            r#"#[ts(rename_all = "camelCase", export )]"#,
            &mut in_ts
        ));
    }

    #[test]
    fn test_export_middle_fails() {
        let mut in_ts = false;
        assert!(check_line_for_ts_export(
            r#"#[ts(rename_all = "camelCase", export, export_to = "x.ts")]"#,
            &mut in_ts
        ));
    }

    #[test]
    fn test_normal_code_with_word_export_passes() {
        let mut in_ts = false;
        assert!(!check_line_for_ts_export(
            r#"pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {"#,
            &mut in_ts
        ));
        assert!(!check_line_for_ts_export(
            r#"    ViewAnswer::<()>::export_all(&cfg)?;"#,
            &mut in_ts
        ));
        assert!(!check_line_for_ts_export(
            r#"let export = true;"#,
            &mut in_ts
        ));
        assert!(!check_line_for_ts_export(
            r#"#[ts(export_to = "x.ts")] // note: do not use ts(export)"#,
            &mut in_ts
        ));
    }
}
