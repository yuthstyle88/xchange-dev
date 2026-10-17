//! Guard test (#2067, stacked on #1641 / PR 2037): ensures `pg_booking_lifecycle` in
//! `xc-booking/src/infrastructure/lifecycle_factory.rs` remains the only production builder of
//! `BookingLifecycle` and is always constructed with slots.
//!
//! No database.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const FACTORY_SUFFIX: &str = "xc-booking/src/infrastructure/lifecycle_factory.rs";
const EXPECTED_FACTORY_CALL_SITES: usize = 5;

fn api_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn normalize_rel(path: &str) -> String {
    path.replace('\\', "/")
}

fn is_factory_file(rel_path: &str) -> bool {
    let norm = normalize_rel(rel_path);
    norm == FACTORY_SUFFIX || norm.ends_with(&format!("/{FACTORY_SUFFIX}"))
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn blank_range(out: &mut [u8], start: usize, end: usize) {
    for b in &mut out[start..end] {
        if *b != b'\n' {
            *b = b' ';
        }
    }
}

/// Replaces `//` line comments, nested `/* ... */` block comments, strings (`"..."`, `b"..."`,
/// `c"..."`), raw strings (`r"..."`, `r#"..."#`, `br#"..."#`, `cr#"..."#` with any number of `#`),
/// and char/byte literals (`'{'`, `'\''`, `b'{'`, told apart from lifetimes `'a`) with spaces while
/// preserving newlines and byte offsets.
fn strip_comments_and_literals(src: &str) -> String {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut out = bytes.to_vec();
    let mut i = 0;

    while i < n {
        // 1. Line comment `// ...`
        if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let start = i;
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            blank_range(&mut out, start, i);
            continue;
        }

        // 2. Nested block comment `/* ... */`
        if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let start = i;
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            blank_range(&mut out, start, i);
            continue;
        }

        // 3. Raw string literal (`r"..."`, `r#"..."#`, `br"..."`, `br#"..."#`, `cr"..."`, `cr#"..."#`)
        let raw_r_pos = if bytes[i] == b'r' && (i == 0 || !is_ident_continue(bytes[i - 1])) {
            Some(i)
        } else if (bytes[i] == b'b' || bytes[i] == b'c')
            && (i == 0 || !is_ident_continue(bytes[i - 1]))
            && i + 1 < n
            && bytes[i + 1] == b'r'
        {
            Some(i + 1)
        } else {
            None
        };

        if let Some(r_pos) = raw_r_pos {
            let mut j = r_pos + 1;
            while j < n && bytes[j] == b'#' {
                j += 1;
            }
            if j < n && bytes[j] == b'"' {
                let hashes = j - (r_pos + 1);
                let start = i;
                i = j + 1;
                while i < n {
                    if bytes[i] == b'"'
                        && i + 1 + hashes <= n
                        && bytes[i + 1..i + 1 + hashes].iter().all(|&b| b == b'#')
                    {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
                blank_range(&mut out, start, i);
                continue;
            }
        }

        // 4. Normal string / byte string / c-string literal (`"..."`, `b"..."`, `c"..."`)
        let is_prefixed_str = (bytes[i] == b'b' || bytes[i] == b'c')
            && (i == 0 || !is_ident_continue(bytes[i - 1]))
            && i + 1 < n
            && bytes[i + 1] == b'"';
        if bytes[i] == b'"' || is_prefixed_str {
            let start = i;
            if is_prefixed_str {
                i += 1;
            }
            i += 1; // opening `"`
            while i < n {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(n);
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            blank_range(&mut out, start, i);
            continue;
        }

        // 5. Char or byte literal (`'c'`, `'\''`, `b'c'`, `b'\''`) vs lifetime (`'a`)
        let quote_pos = if bytes[i] == b'\'' && (i == 0 || !is_ident_continue(bytes[i - 1])) {
            Some(i)
        } else if bytes[i] == b'b'
            && (i == 0 || !is_ident_continue(bytes[i - 1]))
            && i + 1 < n
            && bytes[i + 1] == b'\''
        {
            Some(i + 1)
        } else {
            None
        };

        if let Some(q) = quote_pos {
            if q + 1 < n && bytes[q + 1] == b'\\' {
                let mut k = q + 2;
                let limit = (q + 14).min(n);
                let mut closed = None;
                while k < limit && bytes[k] != b'\n' {
                    if bytes[k] == b'\'' {
                        closed = Some(k);
                        break;
                    }
                    k += 1;
                }
                if let Some(end_quote) = closed {
                    blank_range(&mut out, i, end_quote + 1);
                    i = end_quote + 1;
                    continue;
                }
            } else if q + 1 < n
                && bytes[q + 1] != b'\''
                && bytes[q + 1] != b'\n'
                && bytes[q + 1] != b'\r'
            {
                let ch_len = src[q + 1..].chars().next().map_or(0, char::len_utf8);
                if ch_len > 0 && q + 1 + ch_len < n && bytes[q + 1 + ch_len] == b'\'' {
                    let end = q + 1 + ch_len + 1;
                    blank_range(&mut out, i, end);
                    i = end;
                    continue;
                }
            }
            i = q + 1;
            continue;
        }

        i += 1;
    }

    String::from_utf8(out).expect("stripped source remains valid UTF-8")
}

/// Parses an outer attribute `#[...]` starting at `pos` (where `bytes[pos] == b'#'`) in
/// already-stripped source. Returns `(whitespace_stripped_inner, end_pos)`.
fn parse_outer_attribute(bytes: &[u8], pos: usize) -> Option<(String, usize)> {
    let n = bytes.len();
    if pos >= n || bytes[pos] != b'#' {
        return None;
    }
    let mut i = pos + 1;
    while i < n && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= n || bytes[i] != b'[' {
        return None;
    }
    let inner_start = i + 1;
    let mut depth = 1usize;
    i += 1;
    while i < n && depth > 0 {
        if bytes[i] == b'[' {
            depth += 1;
        } else if bytes[i] == b']' {
            depth -= 1;
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    let inner = &bytes[inner_start..i - 1];
    let normalized: String = inner
        .iter()
        .filter(|b| !b.is_ascii_whitespace())
        .map(|&b| b as char)
        .collect();
    Some((normalized, i))
}

fn read_ident(bytes: &[u8], pos: usize) -> (&str, usize) {
    let n = bytes.len();
    if pos >= n || !is_ident_start(bytes[pos]) {
        return ("", pos);
    }
    let mut end = pos + 1;
    while end < n && is_ident_continue(bytes[end]) {
        end += 1;
    }
    let s = std::str::from_utf8(&bytes[pos..end]).expect("ident is ASCII");
    (s, end)
}

fn skip_ws(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

fn skip_visibility(bytes: &[u8], pos: usize) -> usize {
    let pos = skip_ws(bytes, pos);
    let (word, after_word) = read_ident(bytes, pos);
    if word != "pub" {
        return pos;
    }
    let mut i = skip_ws(bytes, after_word);
    if i < bytes.len() && bytes[i] == b'(' {
        let mut depth = 1usize;
        i += 1;
        while i < bytes.len() && depth > 0 {
            if bytes[i] == b'(' {
                depth += 1;
            } else if bytes[i] == b')' {
                depth -= 1;
            }
            i += 1;
        }
    }
    skip_ws(bytes, i)
}

/// Returns the set of submodule names declared with `#[cfg(test)] mod <name>;` in `parent_src`.
fn cfg_test_submodules(parent_src: &str) -> BTreeSet<String> {
    let stripped = strip_comments_and_literals(parent_src);
    let bytes = stripped.as_bytes();
    let n = bytes.len();
    let mut mods = BTreeSet::new();
    let mut i = 0;

    while i < n {
        if bytes[i] == b'#' {
            let mut cur = i;
            let mut has_cfg_test = false;
            while cur < n {
                cur = skip_ws(bytes, cur);
                if let Some((attr, next_pos)) = parse_outer_attribute(bytes, cur) {
                    if attr == "cfg(test)" {
                        has_cfg_test = true;
                    }
                    cur = next_pos;
                } else {
                    break;
                }
            }
            if has_cfg_test {
                let item_start = skip_visibility(bytes, cur);
                let (kw, after_kw) = read_ident(bytes, item_start);
                if kw == "mod" {
                    let name_start = skip_ws(bytes, after_kw);
                    let (mod_name, after_name) = read_ident(bytes, name_start);
                    let semi = skip_ws(bytes, after_name);
                    if !mod_name.is_empty() && semi < n && bytes[semi] == b';' {
                        mods.insert(mod_name.to_owned());
                        i = semi + 1;
                        continue;
                    }
                }
            }
            if cur > i {
                i = cur;
                continue;
            }
        }
        i += 1;
    }

    mods
}

/// Strips comments, literals, and the `{ ... }` brace span of any inline `#[cfg(test)]` item.
/// A bodiless `#[cfg(test)] use ...;` or `#[cfg(test)] mod x;` skips nothing.
fn production_code(src: &str) -> String {
    let stripped = strip_comments_and_literals(src);
    let bytes = stripped.as_bytes();
    let n = bytes.len();
    let mut out = bytes.to_vec();
    let mut i = 0;

    while i < n {
        if out[i] == b'#' {
            let mut cur = i;
            let mut has_cfg_test = false;
            while cur < n {
                cur = skip_ws(&out, cur);
                if let Some((attr, next_pos)) = parse_outer_attribute(&out, cur) {
                    if attr == "cfg(test)" {
                        has_cfg_test = true;
                    }
                    cur = next_pos;
                } else {
                    break;
                }
            }

            if has_cfg_test {
                let item_start = skip_visibility(&out, cur);
                let (kw, _) = read_ident(&out, item_start);
                if kw == "use" {
                    // Bodiless `#[cfg(test)] use ...;` skips nothing.
                    i = item_start;
                    continue;
                }

                // Scan to the item's opening `{` or terminating `;`.
                let mut j = item_start;
                let mut paren_depth = 0usize;
                while j < n {
                    match out[j] {
                        b'(' => paren_depth += 1,
                        b')' => paren_depth = paren_depth.saturating_sub(1),
                        b';' if paren_depth == 0 => {
                            // Bodiless item (e.g. `#[cfg(test)] mod t;`) skips nothing.
                            i = j + 1;
                            break;
                        }
                        b'{' if paren_depth == 0 => {
                            let brace_start = j;
                            let mut brace_depth = 1usize;
                            j += 1;
                            while j < n && brace_depth > 0 {
                                if out[j] == b'{' {
                                    brace_depth += 1;
                                } else if out[j] == b'}' {
                                    brace_depth -= 1;
                                }
                                j += 1;
                            }
                            blank_range(&mut out, brace_start, j);
                            i = j;
                            break;
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if j >= n {
                    i = n;
                }
                continue;
            }

            if cur > i {
                i = cur;
                continue;
            }
        }
        i += 1;
    }

    String::from_utf8(out).expect("production code remains valid UTF-8")
}

/// Checks Rule (a): whether `rel_path` (or one of its enclosing directories inside its `src` root)
/// is declared `#[cfg(test)] mod <name>;` in its parent module file.
fn is_declared_cfg_test_in_parent(
    rel_path: &str,
    lookup_source: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let norm = normalize_rel(rel_path);
    let path = Path::new(&norm);

    let mut cur: Option<&Path> = Some(path);
    while let Some(p) = cur {
        let Some(file_name) = p.file_name().and_then(|s| s.to_str()) else {
            break;
        };
        if file_name == "src" {
            break;
        }
        let parent_dir = p.parent().unwrap_or_else(|| Path::new(""));

        let mod_name = if p.extension().is_some_and(|ext| ext == "rs") {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem == "mod" || stem == "lib" || stem == "main" {
                cur = p.parent();
                continue;
            }
            stem
        } else {
            file_name
        };

        let mut candidates = vec![
            parent_dir.join("mod.rs"),
            parent_dir.join("lib.rs"),
            parent_dir.join("main.rs"),
        ];
        if let Some(dir_name) = parent_dir.file_name().and_then(|s| s.to_str())
            && dir_name != "src"
            && let Some(grandparent) = parent_dir.parent()
        {
            candidates.push(grandparent.join(format!("{dir_name}.rs")));
        }

        for cand in candidates {
            let cand_str = normalize_rel(&cand.to_string_lossy());
            let cand_key = cand_str.trim_start_matches("./");
            if let Some(parent_src) = lookup_source(cand_key)
                && cfg_test_submodules(&parent_src).contains(mod_name)
            {
                return true;
            }
        }

        cur = p.parent();
    }

    false
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ScanReport {
    rule1_violations: Vec<String>,
    factory_call_sites: Vec<String>,
    scanned_production_files: usize,
}

fn scan_file_map(files: &BTreeMap<String, String>) -> ScanReport {
    let mut report = ScanReport::default();
    let lookup = |rel: &str| -> Option<String> { files.get(rel).cloned() };

    for (rel_path, src) in files {
        if is_declared_cfg_test_in_parent(rel_path, &lookup) {
            continue;
        }
        report.scanned_production_files += 1;
        let prod = production_code(src);

        if is_factory_file(rel_path) {
            continue;
        }

        for (idx, line) in prod.lines().enumerate() {
            let line_no = idx + 1;
            if line.contains("BookingLifecycle::new(") {
                report
                    .rule1_violations
                    .push(format!("{rel_path}:{line_no}: BookingLifecycle::new("));
            }
            if line.contains(".with_slots(") {
                report
                    .rule1_violations
                    .push(format!("{rel_path}:{line_no}: .with_slots("));
            }

            let mut rest = line;
            while let Some(pos) = rest.find("pg_booking_lifecycle(") {
                let prefix = &rest[..pos];
                let is_fn_def = prefix.trim_end().ends_with("fn");
                let prev_ok = prefix
                    .bytes()
                    .next_back()
                    .is_none_or(|b| !is_ident_continue(b));
                if !is_fn_def && prev_ok {
                    report
                        .factory_call_sites
                        .push(format!("{rel_path}:{line_no}"));
                }
                rest = &rest[pos + "pg_booking_lifecycle(".len()..];
            }
        }
    }

    report
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    let mut paths: Vec<PathBuf> = entries
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn workspace_production_roots(api: &Path) -> Vec<PathBuf> {
    let mut roots = vec![api.join("src")];
    let crates_dir = api.join("crates");
    let mut crate_dirs: Vec<PathBuf> = fs::read_dir(&crates_dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", crates_dir.display()))
        .map(|entry| entry.expect("crate entry").path())
        .filter(|path| path.is_dir())
        .collect();
    crate_dirs.sort();
    for crate_dir in crate_dirs {
        let src_dir = crate_dir.join("src");
        if src_dir.is_dir() {
            roots.push(src_dir);
        }
    }
    roots
}

fn load_workspace_files() -> BTreeMap<String, String> {
    let api = api_dir();
    let roots = workspace_production_roots(&api);
    assert!(
        roots.len() >= 10,
        "expected api/src plus crate src roots, found {}",
        roots.len()
    );

    let mut rs_files = Vec::new();
    for root in &roots {
        collect_rs_files(root, &mut rs_files);
    }

    let mut map = BTreeMap::new();
    for path in rs_files {
        let rel = normalize_rel(&path.strip_prefix(&api).unwrap_or(&path).to_string_lossy());
        let content =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        map.insert(rel, content);
    }
    map
}

// ============================================================================
// Behaviour Table Row 1: The real tree after PR 2037
// ============================================================================

#[test]
fn real_tree_builds_booking_lifecycle_only_through_pg_booking_lifecycle() {
    let files = load_workspace_files();
    assert!(
        files.contains_key("src/application/workflows/booking_save_step.rs"),
        "booking_save_step.rs must be present in the scanned tree"
    );
    assert!(
        files.contains_key("crates/xc-booking/src/infrastructure/lifecycle_factory.rs"),
        "lifecycle_factory.rs must be present in the scanned tree"
    );

    let report = scan_file_map(&files);

    assert_eq!(
        report.rule1_violations,
        Vec::<String>::new(),
        "Rule 1 failed: outside {FACTORY_SUFFIX}, no production file may call \
         BookingLifecycle::new( or .with_slots("
    );

    assert_eq!(
        report.factory_call_sites.len(),
        EXPECTED_FACTORY_CALL_SITES,
        "Rule 2 failed: expected exactly {EXPECTED_FACTORY_CALL_SITES} production \
         pg_booking_lifecycle( call sites, found {}: {:?}",
        report.factory_call_sites.len(),
        report.factory_call_sites
    );
}

// ============================================================================
// Behaviour Table Rows 4–11: Fixture self-tests 1–8 of §3
// ============================================================================

#[test]
fn fixture_1_planted_booking_lifecycle_new_in_production_file_is_flagged() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/presentation/http/v3/commands/queue.rs".to_owned(),
        "pub fn handle() {\n    let _ = BookingLifecycle::new(repo);\n}\n".to_owned(),
    );

    let report = scan_file_map(&files);
    assert_eq!(
        report.rule1_violations,
        vec!["src/presentation/http/v3/commands/queue.rs:2: BookingLifecycle::new(".to_owned()]
    );
}

#[test]
fn fixture_2_planted_with_slots_in_production_file_is_flagged() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/presentation/http/rpc.rs".to_owned(),
        "pub fn build(lc: BookingLifecycle) {\n    let _ = lc.with_slots(x);\n}\n".to_owned(),
    );

    let report = scan_file_map(&files);
    assert_eq!(
        report.rule1_violations,
        vec!["src/presentation/http/rpc.rs:2: .with_slots(".to_owned()]
    );
}

#[test]
fn fixture_3_factory_file_is_exempt() {
    let mut files = BTreeMap::new();
    files.insert(
        "crates/xc-booking/src/infrastructure/lifecycle_factory.rs".to_owned(),
        "pub fn pg_booking_lifecycle(pool: PgPool) -> Arc<BookingLifecycle> {\n    \
         Arc::new(BookingLifecycle::new(repo).with_slots(slots))\n\
         }\n"
        .to_owned(),
    );

    let report = scan_file_map(&files);
    assert!(
        report.rule1_violations.is_empty(),
        "factory file must be exempt from Rule 1, got: {:?}",
        report.rule1_violations
    );
    assert!(
        report.factory_call_sites.is_empty(),
        "factory fn definition must not count as a call site, got: {:?}",
        report.factory_call_sites
    );
}

#[test]
fn fixture_4_call_inside_inline_cfg_test_mod_is_not_flagged() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/application/workflows/booking_save_step.rs".to_owned(),
        "pub fn prod() {}\n\
         #[cfg(test)]\n\
         mod t {\n    \
             fn helper() {\n        \
                 let _ = BookingLifecycle::new(repo).with_slots(slots);\n    \
             }\n\
         }\n"
        .to_owned(),
    );

    let report = scan_file_map(&files);
    assert!(
        report.rule1_violations.is_empty(),
        "call inside inline #[cfg(test)] mod t {{ ... }} must not be flagged: {:?}",
        report.rule1_violations
    );
}

#[test]
fn fixture_5_call_after_inline_cfg_test_mod_closing_brace_is_flagged() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/application/workflows/booking_save_step.rs".to_owned(),
        "#[cfg(test)]\n\
         mod t {\n    \
             fn inside_test() {\n        \
                 let _ = BookingLifecycle::new(repo);\n    \
             }\n\
         }\n\
         pub fn after_test_mod() {\n    \
             let _ = BookingLifecycle::new(repo);\n\
         }\n"
        .to_owned(),
    );

    let report = scan_file_map(&files);
    assert_eq!(
        report.rule1_violations,
        vec!["src/application/workflows/booking_save_step.rs:8: BookingLifecycle::new(".to_owned()]
    );
}

#[test]
fn fixture_6_call_inside_file_declared_cfg_test_mod_is_not_flagged() {
    let mut files = BTreeMap::new();
    files.insert(
        "crates/xc-booking/src/application/mod.rs".to_owned(),
        "#[cfg(test)]\nmod t;\nmod prod_child;\n".to_owned(),
    );
    files.insert(
        "crates/xc-booking/src/application/t.rs".to_owned(),
        "pub fn test_helper() {\n    let _ = BookingLifecycle::new(repo);\n}\n".to_owned(),
    );
    files.insert(
        "crates/xc-booking/src/application/prod_child.rs".to_owned(),
        "pub fn prod_fn() {\n    let _ = BookingLifecycle::new(repo);\n}\n".to_owned(),
    );

    let report = scan_file_map(&files);
    assert_eq!(
        report.rule1_violations,
        vec![
            "crates/xc-booking/src/application/prod_child.rs:2: BookingLifecycle::new(".to_owned()
        ],
        "file declared #[cfg(test)] mod t; must be skipped while sibling mod prod_child; is flagged"
    );
}

#[test]
fn fixture_7_call_preceded_by_char_brace_and_raw_string_brace_is_still_flagged() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/presentation/http/kiosk.rs".to_owned(),
        "pub fn tricky() {\n    \
             let ch = '{';\n    \
             let raw = r#\"{\"#;\n    \
             let _ = (ch, raw);\n    \
             let _ = BookingLifecycle::new(repo);\n\
         }\n"
        .to_owned(),
    );

    let report = scan_file_map(&files);
    assert_eq!(
        report.rule1_violations,
        vec!["src/presentation/http/kiosk.rs:5: BookingLifecycle::new(".to_owned()]
    );
}

#[test]
fn fixture_8_call_only_on_comment_line_is_not_flagged() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/presentation/http/rpc.rs".to_owned(),
        "pub fn documented() {\n    \
             // BookingLifecycle::new(repo).with_slots(x)\n    \
             /* outer /* nested BookingLifecycle::new(repo) */ still comment .with_slots(x) */\n\
         }\n"
        .to_owned(),
    );

    let report = scan_file_map(&files);
    assert!(
        report.rule1_violations.is_empty(),
        "commented-out calls must not be flagged: {:?}",
        report.rule1_violations
    );
}

#[test]
fn bodiless_cfg_test_use_skips_nothing() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/presentation/http/v3/commands/bookings.rs".to_owned(),
        "#[cfg(test)]\n\
         use std::collections::{BTreeMap, BTreeSet};\n\
         pub fn leak() {\n    \
             let _ = BookingLifecycle::new(repo);\n\
         }\n"
        .to_owned(),
    );

    let report = scan_file_map(&files);
    assert_eq!(
        report.rule1_violations,
        vec!["src/presentation/http/v3/commands/bookings.rs:4: BookingLifecycle::new(".to_owned()]
    );
}
