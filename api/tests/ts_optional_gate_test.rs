//! Ticket #5303: gate ensuring every field with `skip_serializing_if = "Option::is_none"`
//! in a `#[derive(..., TS, ...)]` struct also carries `#[ts(optional)]`.
//!
//! Without `#[ts(optional)]`, ts-rs generates `T | null`, but the server omits the field
//! on `None`, producing `undefined` on the wire.
//!
//! Ticket #5389: gate ensuring every `Option<...>` field with a `#[ts(type = "X")]`
//! override includes `null` or is `unknown` / `any`. Otherwise ts-rs drops null
//! and generates a required type for an optional field.

use std::fs;
use std::path::{Path, PathBuf};

/// Documented exceptions to the ts(optional) requirement (#5303) or the
/// ts(type) null requirement (#5389).
/// Any new entry requires a documented reason.
const ALLOWED: &[&str] = &[
    // Entries require a documented reason explaining why the DTO field
    // skips serialization when None without being marked #[ts(optional)],
    // or why an Option field's #[ts(type = "...")] override excludes null.
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn scan_roots(manifest_dir: &Path) -> Vec<PathBuf> {
    let mut roots = vec![manifest_dir.join("src")];
    let crates_dir = manifest_dir.join("crates");
    if crates_dir.is_dir() {
        roots.push(crates_dir);
    }
    roots
}

fn has_derive_ts(attr_block: &str) -> bool {
    let mut rest = attr_block;
    while let Some(start) = rest.find("#[derive(") {
        let inside = &rest[start + 9..];
        if let Some(end) = inside.find(']') {
            let derive_inner = &inside[..end];
            if derive_inner
                .split(|c: char| c == ',' || c == '(' || c == ')' || c.is_whitespace())
                .any(|token| token == "TS")
            {
                return true;
            }
            rest = &inside[end..];
        } else {
            break;
        }
    }
    false
}

fn has_ts_optional(attr_block: &str) -> bool {
    let mut rest = attr_block;
    while let Some(start) = rest.find("#[ts(") {
        let inside = &rest[start + 5..];
        if let Some(end) = inside.find(']') {
            let ts_inner = &inside[..end];
            if ts_inner
                .split(|c: char| c == ',' || c == '(' || c == ')' || c == '=' || c.is_whitespace())
                .any(|token| token == "optional")
            {
                return true;
            }
            rest = &inside[end..];
        } else {
            break;
        }
    }
    false
}

fn extract_ts_type(attr_block: &str) -> Option<String> {
    let mut rest = attr_block;
    while let Some(start) = rest.find("#[ts(") {
        let inside = &rest[start + 5..];
        if let Some(end) = inside.find(']') {
            let ts_inner = &inside[..end];
            let mut search = ts_inner;
            while let Some(t_idx) = search.find("type") {
                let before_ok = t_idx == 0 || {
                    let prev = search.as_bytes()[t_idx - 1];
                    prev == b' ' || prev == b',' || prev == b'('
                };
                let after = &search[t_idx + 4..].trim_start();
                if before_ok && after.starts_with('=') {
                    let after_eq = after[1..].trim_start();
                    if let Some(after_quote) = after_eq.strip_prefix('"') {
                        let mut val = String::new();
                        let mut chars = after_quote.chars();
                        while let Some(ch) = chars.next() {
                            if ch == '\\' {
                                if let Some(escaped) = chars.next() {
                                    val.push(escaped);
                                }
                            } else if ch == '"' {
                                return Some(val);
                            } else {
                                val.push(ch);
                            }
                        }
                    }
                }
                search = &search[t_idx + 4..];
            }
            rest = &inside[end..];
        } else {
            break;
        }
    }
    None
}

fn ts_type_satisfies_null(ts_type: &str) -> bool {
    let trimmed = ts_type.trim();
    trimmed == "unknown" || trimmed == "any" || trimmed.split('|').any(|p| p.trim() == "null")
}

fn extract_field(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    let rest = if let Some(stripped) = trimmed.strip_prefix("pub ") {
        stripped.trim_start()
    } else if let Some(stripped) = trimmed.strip_prefix("pub(crate) ") {
        stripped.trim_start()
    } else {
        trimmed
    };
    let (ident, ty) = rest.split_once(':')?;
    let ident = ident.trim();
    if !ident.is_empty() && ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some((ident, ty.trim()))
    } else {
        None
    }
}

fn is_option_type(ty: &str) -> bool {
    let ty = ty.strip_prefix("core::option::").unwrap_or(ty);
    let ty = ty.strip_prefix("std::option::").unwrap_or(ty);
    ty.starts_with("Option<") || ty.starts_with("Option <")
}

fn extract_field_name(line: &str) -> Option<&str> {
    extract_field(line).map(|(name, _)| name)
}

fn check_file(rel_path: &str, content: &str) -> Vec<String> {
    let mut hits = Vec::new();
    let mut in_struct = 0;
    let mut struct_has_ts = false;
    let mut attr_lines = Vec::new();

    for (line_idx, raw_line) in content.lines().enumerate() {
        let line_no = line_idx + 1;
        let line = raw_line.trim();

        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if line.starts_with("#[")
            || (!attr_lines.is_empty()
                && (line.ends_with(']')
                    || (!line.starts_with("pub")
                        && !line.contains(':')
                        && !line.starts_with("struct ")
                        && !line.starts_with("enum "))))
        {
            attr_lines.push(line);
            continue;
        }

        let attr_block = attr_lines.join(" ");

        if line.contains("struct ") {
            let has_ts = has_derive_ts(&attr_block);
            struct_has_ts = has_ts;
            let open_count = line.matches('{').count();
            let close_count = line.matches('}').count();
            in_struct += open_count as isize - close_count as isize;
            attr_lines.clear();
            continue;
        }

        if in_struct > 0 {
            let open_count = line.matches('{').count();
            let close_count = line.matches('}').count();
            in_struct += open_count as isize - close_count as isize;

            if struct_has_ts {
                let is_skipped_option = attr_block.contains("skip_serializing_if")
                    && attr_block.contains("Option::is_none");
                if is_skipped_option && !has_ts_optional(&attr_block) {
                    let field_name = extract_field_name(line).unwrap_or(line);
                    let site = format!("{rel_path}:{line_no} {field_name}");
                    if !ALLOWED.contains(&site.as_str()) {
                        hits.push(site);
                    }
                }
            }
            attr_lines.clear();
            if in_struct <= 0 {
                in_struct = 0;
                struct_has_ts = false;
            }
        } else {
            attr_lines.clear();
        }
    }
    hits
}

fn check_file_ts_types(rel_path: &str, content: &str) -> Vec<String> {
    let mut hits = Vec::new();
    let mut in_struct = 0;
    let mut struct_has_ts = false;
    let mut attr_lines = Vec::new();

    for (line_idx, raw_line) in content.lines().enumerate() {
        let line_no = line_idx + 1;
        let line = raw_line.trim();

        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if line.starts_with("#[")
            || (!attr_lines.is_empty()
                && (line.ends_with(']')
                    || (!line.starts_with("pub")
                        && !line.contains(':')
                        && !line.starts_with("struct ")
                        && !line.starts_with("enum "))))
        {
            attr_lines.push(line);
            continue;
        }

        let attr_block = attr_lines.join(" ");

        if line.contains("struct ") {
            let has_ts = has_derive_ts(&attr_block);
            struct_has_ts = has_ts;
            let open_count = line.matches('{').count();
            let close_count = line.matches('}').count();
            in_struct += open_count as isize - close_count as isize;
            attr_lines.clear();
            continue;
        }

        if in_struct > 0 {
            let open_count = line.matches('{').count();
            let close_count = line.matches('}').count();
            in_struct += open_count as isize - close_count as isize;

            if struct_has_ts
                && let Some((field_name, field_type)) = extract_field(line)
                && is_option_type(field_type)
                && !has_ts_optional(&attr_block)
                && let Some(ts_type) = extract_ts_type(&attr_block)
                && !ts_type_satisfies_null(&ts_type)
            {
                let site = format!("{rel_path}:{line_no} {field_name}");
                if !ALLOWED.contains(&site.as_str()) {
                    hits.push(site);
                }
            }
            attr_lines.clear();
            if in_struct <= 0 {
                in_struct = 0;
                struct_has_ts = false;
            }
        } else {
            attr_lines.clear();
        }
    }
    hits
}

#[test]
fn every_skipped_option_is_ts_optional() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap_or(manifest_dir);

    let mut files = Vec::new();
    for root in scan_roots(manifest_dir) {
        rust_files(&root, &mut files);
    }
    files.sort();
    files.dedup();

    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(repo_root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let content = fs::read_to_string(&file).expect("readable rust source file");
        violations.extend(check_file(&rel, &content));
    }

    assert!(
        violations.is_empty(),
        "every_skipped_option_is_ts_optional failed:\n{}",
        violations.join("\n")
    );
}

#[test]
fn every_option_ts_type_includes_null() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap_or(manifest_dir);

    let mut files = Vec::new();
    for root in scan_roots(manifest_dir) {
        rust_files(&root, &mut files);
    }
    files.sort();
    files.dedup();

    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(repo_root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let content = fs::read_to_string(&file).expect("readable rust source file");
        violations.extend(check_file_ts_types(&rel, &content));
    }

    assert!(
        violations.is_empty(),
        "every_option_ts_type_includes_null failed:\n{}",
        violations.join("\n")
    );
}
