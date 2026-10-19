//! Guard test for LG-6.r4 (#1043, design `docs/design/blueprint/ledger-inventory.md` §4.2.1):
//! fails on any production `PgShiftRepository::new(` outside `ledger_shift_repository.rs`, so a
//! new caller cannot silently build a repository with no `ShiftBalances` port.
//!
//! No database.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const FACTORY_SUFFIX: &str = "xc-cash/src/infrastructure/ledger/ledger_shift_repository.rs";

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
            i += 1;
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
/// Exclusion (b): skips an item's span only when `{` comes before `;` at depth 0; a `;` first
/// means the item has no body (`#[cfg(test)] use foo;`, `#[cfg(test)] mod t;`) and nothing is skipped.
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
                    i = item_start;
                    continue;
                }

                let mut j = item_start;
                let mut paren_depth = 0usize;
                while j < n {
                    match out[j] {
                        b'(' => paren_depth += 1,
                        b')' => paren_depth = paren_depth.saturating_sub(1),
                        b';' if paren_depth == 0 => {
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

fn scan_file_map(files: &BTreeMap<String, String>) -> Vec<String> {
    let mut violations = Vec::new();
    let lookup = |rel: &str| -> Option<String> { files.get(rel).cloned() };

    for (rel_path, src) in files {
        if is_declared_cfg_test_in_parent(rel_path, &lookup) {
            continue;
        }
        if is_factory_file(rel_path) {
            continue;
        }

        let prod = production_code(src);
        for (idx, line) in prod.lines().enumerate() {
            if line.contains("PgShiftRepository::new(") {
                violations.push(format!("{rel_path}:{}: PgShiftRepository::new(", idx + 1));
            }
        }
    }

    violations
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

#[test]
fn real_tree_has_no_portless_shift_repository_in_production() {
    let files = load_workspace_files();
    assert!(
        files.contains_key("src/application/workflows/booking_save_step.rs"),
        "booking_save_step.rs must be present in the scanned tree"
    );
    assert!(
        files.contains_key("crates/xc-shift/src/application/branch_day/tests.rs"),
        "branch_day/tests.rs must be present in the scanned tree"
    );
    assert!(
        files.contains_key("crates/xc-cash/src/infrastructure/ledger/ledger_shift_repository.rs"),
        "ledger_shift_repository.rs must be present in the scanned tree"
    );

    let violations = scan_file_map(&files);
    assert!(
        violations.is_empty(),
        "No production file outside {FACTORY_SUFFIX} may call PgShiftRepository::new( \
         (use ledger_shift_repository(pool) instead, design §4.2.1). Found:\n{}",
        violations.join("\n")
    );
}

#[test]
fn g1_file_reached_only_by_generated_path_or_include_is_caught() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/presentation/http/v3/views/admin_shift_state.rs".to_owned(),
        "pub fn register() {\n    let _ = PgShiftRepository::new(pool);\n}\n".to_owned(),
    );
    assert_eq!(
        scan_file_map(&files),
        vec![
            "src/presentation/http/v3/views/admin_shift_state.rs:2: PgShiftRepository::new("
                .to_owned()
        ]
    );
}

#[test]
fn g2_char_brace_and_raw_string_brace_before_call_still_caught() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/presentation/http/rpc.rs".to_owned(),
        "pub fn build() {\n    let c = '{';\n    let r = r#\"{\"#;\n    let _ = (c, r);\n    let _ = PgShiftRepository::new(pool);\n}\n".to_owned(),
    );
    assert_eq!(
        scan_file_map(&files),
        vec!["src/presentation/http/rpc.rs:5: PgShiftRepository::new(".to_owned()]
    );
}

#[test]
fn g3_call_after_inline_cfg_test_mod_closing_brace_is_caught() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/application/workflows/booking_save_step.rs".to_owned(),
        "#[cfg(test)]\nmod t {\n    fn helper() {\n        let _ = PgShiftRepository::new(pool);\n    }\n}\npub fn leak() {\n    let _ = PgShiftRepository::new(pool);\n}\n".to_owned(),
    );
    assert_eq!(
        scan_file_map(&files),
        vec![
            "src/application/workflows/booking_save_step.rs:8: PgShiftRepository::new(".to_owned()
        ]
    );
}

#[test]
fn g4_call_inside_inline_cfg_test_mod_is_not_caught() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/application/workflows/booking_save_step.rs".to_owned(),
        "pub fn prod() {}\n#[cfg(test)]\nmod t {\n    fn helper() {\n        let _ = PgShiftRepository::new(pool);\n    }\n}\n".to_owned(),
    );
    assert!(scan_file_map(&files).is_empty());
}

#[test]
fn g5_file_declared_cfg_test_mod_in_parent_is_not_caught_but_without_attr_is_caught() {
    let mut files = BTreeMap::new();
    files.insert(
        "crates/xc-shift/src/application/branch_day/mod.rs".to_owned(),
        "#[cfg(test)]\nmod t;\nmod prod_child;\n".to_owned(),
    );
    files.insert(
        "crates/xc-shift/src/application/branch_day/t.rs".to_owned(),
        "pub fn test_helper() {\n    let _ = PgShiftRepository::new(pool);\n}\n".to_owned(),
    );
    files.insert(
        "crates/xc-shift/src/application/branch_day/prod_child.rs".to_owned(),
        "pub fn prod_fn() {\n    let _ = PgShiftRepository::new(pool);\n}\n".to_owned(),
    );
    assert_eq!(
        scan_file_map(&files),
        vec![
            "crates/xc-shift/src/application/branch_day/prod_child.rs:2: PgShiftRepository::new("
                .to_owned()
        ]
    );
}

#[test]
fn g6_call_under_crate_src_root_is_caught() {
    let mut files = BTreeMap::new();
    files.insert(
        "crates/xc-cash/src/application/count/count_service.rs".to_owned(),
        "pub fn pg() {\n    let _ = PgShiftRepository::new(pool);\n}\n".to_owned(),
    );
    assert_eq!(
        scan_file_map(&files),
        vec![
            "crates/xc-cash/src/application/count/count_service.rs:2: PgShiftRepository::new("
                .to_owned()
        ]
    );
}

#[test]
fn g7_bodiless_cfg_test_item_followed_by_prod_call_is_caught() {
    let mut files = BTreeMap::new();
    files.insert(
        "src/presentation/http/rpc.rs".to_owned(),
        "#[cfg(test)]\nuse foo;\npub fn prod() {\n    let _ = PgShiftRepository::new(pool);\n}\n"
            .to_owned(),
    );
    assert_eq!(
        scan_file_map(&files),
        vec!["src/presentation/http/rpc.rs:4: PgShiftRepository::new(".to_owned()]
    );
}

#[test]
fn g8_call_inside_ledger_shift_repository_is_not_caught() {
    let mut files = BTreeMap::new();
    files.insert(
        "crates/xc-cash/src/infrastructure/ledger/ledger_shift_repository.rs".to_owned(),
        "pub fn ledger_shift_repository() {\n    let _ = PgShiftRepository::new(pool);\n}\n"
            .to_owned(),
    );
    assert!(scan_file_map(&files).is_empty());
}
