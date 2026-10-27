//! Ticket #5303: gate ensuring every field with `skip_serializing_if = "Option::is_none"`
//! in a `#[derive(..., TS, ...)]` struct also carries `#[ts(optional)]`.
//!
//! Without `#[ts(optional)]`, ts-rs generates `T | null`, but the server omits the field
//! on `None`, producing `undefined` on the wire.

use std::fs;
use std::path::{Path, PathBuf};

/// Documented exceptions to the ts(optional) requirement.
/// Any new entry requires a documented reason.
const ALLOWED: &[&str] = &[
    // Entries require a documented reason explaining why the DTO field
    // skips serialization when None without being marked #[ts(optional)].
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

fn extract_field_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let rest = if let Some(stripped) = trimmed.strip_prefix("pub ") {
        stripped.trim_start()
    } else if let Some(stripped) = trimmed.strip_prefix("pub(crate) ") {
        stripped.trim_start()
    } else {
        trimmed
    };
    let (ident, _) = rest.split_once(':')?;
    let ident = ident.trim();
    if !ident.is_empty() && ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(ident)
    } else {
        None
    }
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
