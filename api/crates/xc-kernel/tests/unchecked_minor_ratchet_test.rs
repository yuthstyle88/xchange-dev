//! Ratchet: production call sites of `.minor()` (today `Money::minor`, which wraps past
//! `i64::MAX` hundredths) may only go down. Written by the ticket author (claude/d7aa64).
//!
//! Ceiling lowers in each per-file migration PR; the last ticket removes the unchecked fn and this test.
//!
//! How it counts: every `.rs` file under `api/src` and `api/crates/*/src`, skipping any
//! `tests/` directory, any file module declared under `#[cfg(test)]` (`#[cfg(test)] mod x;`)
//! and the body of any inline `#[cfg(test)] mod x { … }`. Text after `//` on a line is
//! ignored. A site is the exact text `.minor()` (so `.minor_i128()`, `.minor_unit()` and a
//! field `.minor` do not match), or a path reference `Money::minor` followed by a character
//! that cannot continue an identifier (`.map(Money::minor)`).
//!
//! The matcher reads text, not types: `MoneyDelta::minor()` (an `i128`, safe) is also
//! `.minor()` and is counted. Those sites set the floor this ceiling can reach before
//! `Money::minor` is removed; removing it makes the compiler the final gate. Not seen: a
//! call split across lines as `.minor` then `()`, a `use`-renamed method path, and a call built by a macro.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Sites on origin/main when this test was written.
const CEILING: usize = 66;

fn api_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn source_roots() -> Vec<PathBuf> {
    let root = api_root();
    let mut roots = vec![root.join("src")];
    let crates = fs::read_dir(root.join("crates")).expect("api/crates exists");
    for entry in crates {
        let src = entry.expect("a dir entry").path().join("src");
        if src.is_dir() {
            roots.push(src);
        }
    }
    roots.sort();
    roots
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .expect("a readable dir")
        .map(|e| e.expect("a dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Lines of `file` outside inline `#[cfg(test)] mod … { … }` blocks, plus the paths of
/// file modules it declares under `#[cfg(test)]`.
fn production_lines(file: &Path, text: &str) -> (Vec<(usize, String)>, Vec<PathBuf>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();
    let mut test_files = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let code = strip_comment(lines[i]).trim();
        if code == "#[cfg(test)]" {
            let mut j = i + 1;
            while j < lines.len() && strip_comment(lines[j]).trim().starts_with("#[") {
                j += 1;
            }
            let decl = lines.get(j).map(|l| strip_comment(l).trim()).unwrap_or("");
            let decl = decl.strip_prefix("pub ").unwrap_or(decl);
            if let Some(rest) = decl.strip_prefix("mod ") {
                if let Some(name) = rest.strip_suffix(';') {
                    let dir = module_dir(file);
                    test_files.push(dir.join(format!("{}.rs", name.trim())));
                    test_files.push(dir.join(name.trim()).join("mod.rs"));
                    i = j + 1;
                    continue;
                }
                if rest.ends_with('{') {
                    let mut depth = 0i32;
                    let mut k = j;
                    while k < lines.len() {
                        let l = strip_comment(lines[k]);
                        depth += l.matches('{').count() as i32;
                        depth -= l.matches('}').count() as i32;
                        k += 1;
                        if depth <= 0 {
                            break;
                        }
                    }
                    i = k;
                    continue;
                }
            }
        }
        kept.push((i + 1, lines[i].to_string()));
        i += 1;
    }
    (kept, test_files)
}

/// The directory a file's child modules live in.
fn module_dir(file: &Path) -> PathBuf {
    let parent = file.parent().expect("a parent").to_path_buf();
    let stem = file.file_stem().expect("a stem").to_string_lossy();
    if stem == "mod" || stem == "lib" || stem == "main" {
        parent
    } else {
        parent.join(stem.as_ref())
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn count_in(code: &str, needle: &str) -> usize {
    let mut n = 0;
    for (at, _) in code.match_indices(needle) {
        let before_ok = code[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident_char(c));
        let after_ok = code[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            n += 1;
        }
    }
    n
}

/// `.minor()` exactly, plus `Money::minor` used as a path.
fn count_minor(code: &str) -> usize {
    code.matches(".minor()").count() + count_in(code, "Money::minor")
}

fn sites() -> BTreeMap<String, usize> {
    let mut files = Vec::new();
    for root in source_roots() {
        rust_files(&root, &mut files);
    }
    let mut parsed = Vec::new();
    let mut excluded = BTreeSet::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("utf-8 source");
        let (kept, test_files) = production_lines(file, &text);
        excluded.extend(test_files);
        parsed.push((file.clone(), kept));
    }
    let root = api_root();
    let mut found = BTreeMap::new();
    for (file, kept) in parsed {
        if excluded.contains(&file) {
            continue;
        }
        for (line_no, line) in kept {
            let n = count_minor(strip_comment(&line));
            if n > 0 {
                let rel = file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                found.insert(format!("{rel}:{line_no}"), n);
            }
        }
    }
    found
}

#[test]
fn unchecked_minor_call_sites_do_not_grow() {
    let found = sites();
    let total: usize = found.values().sum();
    let listing: Vec<String> = found.iter().map(|(k, n)| format!("  {k} ×{n}")).collect();
    assert!(
        total <= CEILING,
        "{total} call sites of .minor(), ceiling {CEILING}. Use Money::try_minor (or \
         minor_i128) in new code.\n{}",
        listing.join("\n")
    );
    if total < CEILING {
        eprintln!("ratchet: {total} sites < ceiling {CEILING}; lower CEILING to {total}");
    }
}

#[test]
fn the_matcher_sees_minor_exactly() {
    assert_eq!(count_minor("let x = m.minor();"), 1);
    assert_eq!(count_minor("Decimal::new(a.minor(), 2) + b.minor()"), 2);
    assert_eq!(count_minor(".map(Money::minor)"), 1);
    assert_eq!(count_minor("m.minor_i128()"), 0);
    assert_eq!(count_minor("u.minor_unit().get()"), 0);
    assert_eq!(count_minor("let minor = x.minor;"), 0);
    assert_eq!(count_minor("Money::minor_i128(m)"), 0);
    assert_eq!(count_minor("fn minor(v: Decimal)"), 0);
    let (kept, _) = production_lines(
        Path::new("a/b.rs"),
        "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() { m.minor(); }\n}\nfn c() {}\n",
    );
    let kept: Vec<usize> = kept.iter().map(|(n, _)| *n).collect();
    assert_eq!(kept, vec![1, 6]);
    let (_, test_files) = production_lines(Path::new("x/y/mod.rs"), "#[cfg(test)]\nmod tests;\n");
    assert!(test_files.contains(&PathBuf::from("x/y/tests.rs")));
}
