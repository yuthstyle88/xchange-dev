//! #2111 (part of #2108): source-scan ratchet stopping new "silent zero" money fallbacks
//! (`unwrap_or(Money::ZERO)`, `Decimal::from_str(..).unwrap_or_default()`, etc.) across
//! `api/src` and `api/crates/*/src`.
//!
//! Limit: a hit split across lines (e.g. `.unwrap_or(` on one line and `Money::ZERO)` on the
//! next line) is not caught by the line-oriented matcher; `rustfmt` keeps these short calls on
//! one line.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Pattern {
    R1,
    R2,
    R3,
    R4,
    R5,
}

impl Pattern {
    const ALL: [Pattern; 5] = [
        Pattern::R1,
        Pattern::R2,
        Pattern::R3,
        Pattern::R4,
        Pattern::R5,
    ];
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pattern::R1 => write!(f, "R1"),
            Pattern::R2 => write!(f, "R2"),
            Pattern::R3 => write!(f, "R3"),
            Pattern::R4 => write!(f, "R4"),
            Pattern::R5 => write!(f, "R5"),
        }
    }
}

const R1_CEILINGS: &[(&str, usize)] = &[
    ("crates/xc-cash/src/application/count/count_service.rs", 3),
    (
        "crates/xc-cash/src/application/handover/handover_service.rs",
        1,
    ),
    ("crates/xc-cash/src/infrastructure/ledger_postings.rs", 2),
    (
        "crates/xc-cash/src/infrastructure/transfer_repository.rs",
        1,
    ),
    (
        "crates/xc-exchange/src/domain/exchange/value_objects/transaction_form.rs",
        1,
    ),
    (
        "crates/xc-marketing/src/domain/benefit/kinds/fee_waiver.rs",
        6,
    ),
    (
        "crates/xc-marketing/src/domain/benefit/kinds/rate_improvement.rs",
        6,
    ),
    ("crates/xc-marketing/src/domain/engine/engine.rs", 1),
    ("crates/xc-marketing/src/domain/engine/rate_floor.rs", 3),
    (
        "crates/xc-marketing/src/infrastructure/pg_performance.rs",
        1,
    ),
    (
        "crates/xc-shift/src/domain/shift/value_objects/balances.rs",
        1,
    ),
    (
        "crates/xc-shift/src/domain/shift/value_objects/stock_delta.rs",
        2,
    ),
    ("src/application/workflows/exchange/quote.rs", 2),
    ("src/infrastructure/executive/alert_facts.rs", 1),
    ("src/presentation/http/v3/commands/rate_policy_save.rs", 2),
    ("src/presentation/http/v3/views/admin_cash.rs", 4),
    ("src/presentation/http/v3/views/admin_shifts.rs", 1),
];

const R3_CEILINGS: &[(&str, usize)] = &[
    (
        "crates/xc-marketing/src/domain/benefit/kinds/fee_waiver.rs",
        4,
    ),
    (
        "crates/xc-marketing/src/domain/benefit/kinds/rate_improvement.rs",
        4,
    ),
    ("src/application/workflows/exchange/quote.rs", 1),
    ("src/presentation/pages.rs", 3),
];

const R4_CEILINGS: &[(&str, usize)] = &[
    (
        "crates/xc-cash/src/infrastructure/transfer_repository.rs",
        1,
    ),
    (
        "crates/xc-exchange/src/domain/exchange/value_objects/transaction_form.rs",
        2,
    ),
    (
        "crates/xc-marketing/src/domain/benefit/kinds/fee_waiver.rs",
        2,
    ),
    (
        "crates/xc-marketing/src/domain/benefit/kinds/rate_improvement.rs",
        2,
    ),
    ("crates/xc-marketing/src/domain/engine/rate_floor.rs", 3),
];

fn ceiling_for(pattern: Pattern, file: &str) -> usize {
    let table: &[(&str, usize)] = match pattern {
        Pattern::R1 => R1_CEILINGS,
        Pattern::R3 => R3_CEILINGS,
        Pattern::R4 => R4_CEILINGS,
        Pattern::R2 | Pattern::R5 => &[],
    };
    table
        .iter()
        .find_map(|(path, c)| (*path == file).then_some(*c))
        .unwrap_or(0)
}

fn is_word_or_colon(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == ':'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn matches_qualified_type(s: &str, allowed_types: &[&str]) -> bool {
    for &ty in allowed_types {
        if s == ty {
            return true;
        }
        if let Some(prefix) = s.strip_suffix(ty)
            && let Some(path) = prefix.strip_suffix("::")
            && !path.is_empty()
            && path.chars().all(is_word_or_colon)
        {
            return true;
        }
    }
    false
}

/// R1: `\.unwrap_or\(\s*(?:[\w:]*::)?(?:Money|Decimal|MoneyDelta|ForeignAmount|Rate|RatePercent)::ZERO\s*\)`
fn count_r1(line: &str) -> usize {
    const NEEDLE: &str = ".unwrap_or(";
    const TYPES: &[&str] = &[
        "Money",
        "Decimal",
        "MoneyDelta",
        "ForeignAmount",
        "Rate",
        "RatePercent",
    ];
    let mut count = 0;
    let mut rest = line;
    while let Some(pos) = rest.find(NEEDLE) {
        let after = &rest[pos + NEEDLE.len()..];
        if let Some(close) = after.find(')') {
            let inner = after[..close].trim();
            if let Some(ty_path) = inner.strip_suffix("::ZERO")
                && matches_qualified_type(ty_path, TYPES)
            {
                count += 1;
                rest = &after[close + 1..];
                continue;
            }
        }
        rest = &rest[pos + NEEDLE.len()..];
    }
    count
}

/// R2: `\.unwrap_or_else\(\s*\|[^|]*\|\s*(?:(?:[\w:]*::)?(?:Money|Decimal|MoneyDelta|ForeignAmount)::ZERO|0(?:_?i64|_?i128)?)\s*\)`
fn count_r2(line: &str) -> usize {
    const NEEDLE: &str = ".unwrap_or_else(";
    const TYPES: &[&str] = &["Money", "Decimal", "MoneyDelta", "ForeignAmount"];
    let mut count = 0;
    let mut rest = line;
    while let Some(pos) = rest.find(NEEDLE) {
        let after = &rest[pos + NEEDLE.len()..];
        let trimmed = after.trim_start();
        if let Some(after_open_pipe) = trimmed.strip_prefix('|')
            && let Some(close_pipe) = after_open_pipe.find('|')
        {
            let after_pipes = &after_open_pipe[close_pipe + 1..];
            if let Some(close_paren) = after_pipes.find(')') {
                let body = after_pipes[..close_paren].trim();
                let is_zero_int = matches!(body, "0" | "0i64" | "0_i64" | "0i128" | "0_i128");
                let is_zero_type = body
                    .strip_suffix("::ZERO")
                    .is_some_and(|ty_path| matches_qualified_type(ty_path, TYPES));
                if is_zero_int || is_zero_type {
                    count += 1;
                    rest = &after_pipes[close_paren + 1..];
                    continue;
                }
            }
        }
        rest = &rest[pos + NEEDLE.len()..];
    }
    count
}

/// Checks if `s` starts with `\)\s*\.(?:unwrap_or_default\(\)|unwrap_or\()` (for R3)
/// or `\)\s*\.unwrap_or(?:_default\(\)|\(|_else\()` (for R4), returning the byte length consumed.
fn match_unwrap_chain(s: &str, allow_else: bool) -> Option<usize> {
    let after_paren = s.strip_prefix(')')?;
    let trimmed = after_paren.trim_start();
    let ws_len = after_paren.len() - trimmed.len();
    for suffix in [".unwrap_or_default()", ".unwrap_or(", ".unwrap_or_else("] {
        if suffix == ".unwrap_or_else(" && !allow_else {
            continue;
        }
        if trimmed.starts_with(suffix) {
            return Some(1 + ws_len + suffix.len());
        }
    }
    None
}

/// Finds the start and opening-paren index of the next R3 call prefix in `s`:
/// `(?:Decimal::from_str(?:_exact)?|parse::<\s*Decimal\s*>)(`
fn next_r3_prefix(s: &str) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for literal in ["Decimal::from_str_exact(", "Decimal::from_str("] {
        if let Some(pos) = s.find(literal) {
            let before_ok = s[..pos]
                .chars()
                .next_back()
                .is_none_or(|c| !is_ident_char(c));
            if before_ok {
                let open_end = pos + literal.len();
                if best.is_none_or(|(b_pos, _)| pos < b_pos) {
                    best = Some((pos, open_end));
                }
            }
        }
    }
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find("parse::<") {
        let pos = search_from + rel;
        let before_ok = s[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident_char(c));
        let after_turbo = &s[pos + "parse::<".len()..];
        if before_ok
            && let Some(gt) = after_turbo.find('>')
            && after_turbo[..gt].trim() == "Decimal"
            && after_turbo[gt + 1..].starts_with('(')
        {
            let open_end = pos + "parse::<".len() + gt + 2;
            if best.is_none_or(|(b_pos, _)| pos < b_pos) {
                best = Some((pos, open_end));
            }
            break;
        }
        search_from = pos + "parse::<".len();
    }
    best
}

/// R3: `(?:Decimal::from_str(?:_exact)?|parse::<\s*Decimal\s*>)\([^;]*?\)\s*\.(?:unwrap_or_default\(\)|unwrap_or\()`
fn count_r3(line: &str) -> usize {
    let mut count = 0;
    let mut rest = line;
    while let Some((_pos, open_end)) = next_r3_prefix(rest) {
        let after_open = &rest[open_end..];
        let limit = after_open.find(';').unwrap_or(after_open.len());
        let mut matched_end = None;
        for (idx, ch) in after_open[..limit].char_indices() {
            if ch == ')'
                && let Some(consumed) = match_unwrap_chain(&after_open[idx..], false)
            {
                matched_end = Some(idx + consumed);
                break;
            }
        }
        if let Some(end) = matched_end {
            count += 1;
            rest = &after_open[end..];
        } else {
            rest = after_open;
        }
    }
    count
}

/// Finds the start and opening-paren index of the next R4 call prefix in `s`:
/// `(?:Money|ForeignAmount|Rate|MoneyDelta)::(?:parse|from_minor|try_from_minor_i128)(`
fn next_r4_prefix(s: &str) -> Option<(usize, usize)> {
    const TYPES: &[&str] = &["Money", "ForeignAmount", "Rate", "MoneyDelta"];
    const METHODS: &[&str] = &["parse", "from_minor", "try_from_minor_i128"];
    let mut best: Option<(usize, usize)> = None;
    for ty in TYPES {
        for method in METHODS {
            let pat = format!("{ty}::{method}(");
            let mut search_from = 0;
            while let Some(rel) = s[search_from..].find(&pat) {
                let pos = search_from + rel;
                let before_ok = s[..pos]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !is_ident_char(c));
                if before_ok {
                    let open_end = pos + pat.len();
                    if best.is_none_or(|(b_pos, _)| pos < b_pos) {
                        best = Some((pos, open_end));
                    }
                    break;
                }
                search_from = pos + pat.len();
            }
        }
    }
    best
}

/// R4: `(?:Money|ForeignAmount|Rate|MoneyDelta)::(?:parse|from_minor|try_from_minor_i128)\([^;]*\)\s*\.unwrap_or(?:_default\(\)|\(|_else\()`
fn count_r4(line: &str) -> usize {
    let mut count = 0;
    let mut rest = line;
    while let Some((_pos, open_end)) = next_r4_prefix(rest) {
        let after_open = &rest[open_end..];
        let limit = after_open.find(';').unwrap_or(after_open.len());
        let mut matched_end = None;
        for (idx, ch) in after_open[..limit].char_indices() {
            if ch == ')'
                && let Some(consumed) = match_unwrap_chain(&after_open[idx..], true)
            {
                matched_end = Some(idx + consumed);
            }
        }
        if let Some(end) = matched_end {
            count += 1;
            rest = &after_open[end..];
        } else {
            rest = after_open;
        }
    }
    count
}

fn is_r5_unwrap_arg(inner: &str) -> bool {
    if inner == "Decimal::from(0)" || inner == "dec!(0)" {
        return true;
    }
    if let Some(zeros) = inner
        .strip_prefix("dec!(0.")
        .and_then(|s| s.strip_suffix(')'))
        && !zeros.is_empty()
        && zeros.chars().all(|c| c == '0')
    {
        return true;
    }
    if let Some(scale) = inner
        .strip_prefix("Decimal::new(0,")
        .and_then(|s| s.strip_suffix(')'))
    {
        let scale = scale.trim_start();
        if !scale.is_empty() && scale.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// R5:
/// - `\.unwrap_or\(\s*(?:Decimal::new\(0,\s*\d+\)|dec!\(0(?:\.0+)?\)|Decimal::from\(0\))\s*\)`
/// - `\.map_or\(\s*(?:Money|Decimal|MoneyDelta)::ZERO\s*,`
fn count_r5(line: &str) -> usize {
    let mut count = 0;

    const UNWRAP_NEEDLE: &str = ".unwrap_or(";
    let mut rest = line;
    while let Some(pos) = rest.find(UNWRAP_NEEDLE) {
        let after = &rest[pos + UNWRAP_NEEDLE.len()..];
        // The inner call itself has a closing `)`, so look for the outer `)` after the inner `)`.
        let mut matched = false;
        for (idx, ch) in after.char_indices() {
            if ch == ')' {
                let candidate = after[..idx].trim();
                if is_r5_unwrap_arg(candidate) {
                    count += 1;
                    rest = &after[idx + 1..];
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            rest = &rest[pos + UNWRAP_NEEDLE.len()..];
        }
    }

    const MAP_NEEDLE: &str = ".map_or(";
    let mut rest = line;
    while let Some(pos) = rest.find(MAP_NEEDLE) {
        let after = &rest[pos + MAP_NEEDLE.len()..];
        if let Some(comma) = after.find(',') {
            let arg = after[..comma].trim();
            if matches!(arg, "Money::ZERO" | "Decimal::ZERO" | "MoneyDelta::ZERO") {
                count += 1;
                rest = &after[comma + 1..];
                continue;
            }
        }
        rest = &rest[pos + MAP_NEEDLE.len()..];
    }

    count
}

fn strip_line_comment(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

fn hits(line: &str) -> [usize; 5] {
    if line.trim_start().starts_with("//") {
        return [0; 5];
    }
    let code = strip_line_comment(line);
    [
        count_r1(code),
        count_r2(code),
        count_r3(code),
        count_r4(code),
        count_r5(code),
    ]
}

fn is_mod_block_start(trimmed: &str) -> bool {
    let rest = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
    if let Some(after_mod) = rest.strip_prefix("mod ") {
        after_mod.contains('{')
    } else {
        false
    }
}

/// Filters `text` into kept non-test, non-comment lines according to §7–9:
/// - skip lines whose trimmed start is `//`;
/// - skip every line of a `#[cfg(test)]` item: from the attribute, through the following
///   `mod name {` block to its matching `}` (counting braces), or, if the next non-attribute
///   line is not a `mod … {`, that one item line.
fn kept_lines(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if trimmed.starts_with("//") {
            i += 1;
            continue;
        }
        let code_trimmed = strip_line_comment(lines[i]).trim();
        if code_trimmed == "#[cfg(test)]" {
            let mut j = i + 1;
            while j < lines.len() {
                let t = lines[j].trim_start();
                if t.starts_with("//") || strip_line_comment(lines[j]).trim().starts_with("#[") {
                    j += 1;
                } else {
                    break;
                }
            }
            if j >= lines.len() {
                break;
            }
            let item_line = strip_line_comment(lines[j]).trim();
            if is_mod_block_start(item_line) {
                let mut depth = 0i32;
                let mut k = j;
                while k < lines.len() {
                    let l = strip_line_comment(lines[k]);
                    depth += l.matches('{').count() as i32;
                    depth -= l.matches('}').count() as i32;
                    k += 1;
                    if depth <= 0 {
                        break;
                    }
                }
                i = k;
                continue;
            } else {
                i = j + 1;
                continue;
            }
        }
        kept.push(lines[i]);
        i += 1;
    }
    kept
}

fn hits_in_source(text: &str) -> [usize; 5] {
    let mut total = [0usize; 5];
    for line in kept_lines(text) {
        let h = hits(line);
        for k in 0..5 {
            total[k] += h[k];
        }
    }
    total
}

fn api_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .expect("readable directory")
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if name == "tests.rs" || name.ends_with("_test.rs") {
                continue;
            }
            out.push(path);
        }
    }
}

fn scanned_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let api_src = root.join("src");
    if api_src.is_dir() {
        collect_rs_files(&api_src, &mut files);
    }
    let crates_dir = root.join("crates");
    if crates_dir.is_dir() {
        let mut crate_entries: Vec<PathBuf> = fs::read_dir(&crates_dir)
            .expect("read api/crates")
            .map(|e| e.expect("crate entry").path())
            .collect();
        crate_entries.sort();
        for crate_dir in crate_entries {
            let src = crate_dir.join("src");
            if src.is_dir() {
                collect_rs_files(&src, &mut files);
            }
        }
    }
    files
}

fn scan(root: &Path) -> (BTreeMap<(Pattern, String), usize>, BTreeSet<String>) {
    let files = scanned_files(root);
    let mut counts = BTreeMap::new();
    let mut visited = BTreeSet::new();
    for file in files {
        let rel = file
            .strip_prefix(root)
            .expect("under api root")
            .to_string_lossy()
            .replace('\\', "/");
        visited.insert(rel.clone());
        let text = fs::read_to_string(&file).expect("utf-8 rust source");
        let file_hits = hits_in_source(&text);
        for (idx, pat) in Pattern::ALL.into_iter().enumerate() {
            if file_hits[idx] > 0 {
                counts.insert((pat, rel.clone()), file_hits[idx]);
            }
        }
    }
    (counts, visited)
}

// ------------------------------------------------------------ T1..T6

#[test]
fn ceilings_hold() {
    let root = api_root();
    let (counts, _) = scan(&root);

    for pat in [Pattern::R1, Pattern::R3, Pattern::R4] {
        let total: usize = counts
            .iter()
            .filter_map(|((p, _), n)| (*p == pat).then_some(*n))
            .sum();
        println!("=== {pat} — {total} ===");
        for ((p, file), n) in &counts {
            if *p == pat {
                println!("{file} {n}");
            }
        }
    }

    let mut all_files: BTreeSet<(Pattern, String)> = counts.keys().cloned().collect();
    for &(file, _) in R1_CEILINGS {
        all_files.insert((Pattern::R1, file.to_string()));
    }
    for &(file, _) in R3_CEILINGS {
        all_files.insert((Pattern::R3, file.to_string()));
    }
    for &(file, _) in R4_CEILINGS {
        all_files.insert((Pattern::R4, file.to_string()));
    }

    let mut violations = Vec::new();
    for (pat, file) in all_files {
        if matches!(pat, Pattern::R2 | Pattern::R5) {
            continue;
        }
        let measured = counts.get(&(pat, file.clone())).copied().unwrap_or(0);
        let ceiling = ceiling_for(pat, &file);
        if measured > ceiling {
            violations.push(format!(
                "{pat} {file}: measured {measured} > ceiling {ceiling} — a money value must not fall back to zero silently — refuse (see #2108) or, if zero is truly right, raise the ceiling with a reason in the PR"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "money silent-zero ratchet failed:\n{}",
        violations.join("\n")
    );
}

#[test]
fn hard_zero_patterns_have_no_hits() {
    let root = api_root();
    let (counts, _) = scan(&root);
    let mut violations = Vec::new();
    for ((pat, file), measured) in &counts {
        if matches!(pat, Pattern::R2 | Pattern::R5) && *measured > 0 {
            violations.push(format!(
                "{pat} {file}: measured {measured} > ceiling 0 (hard zero) — a money value must not fall back to zero silently — refuse (see #2108) or, if zero is truly right, raise the ceiling with a reason in the PR"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "hard-zero patterns R2/R5 had hits:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_matcher_finds_each_pattern() {
    // R1: each type name (and qualified path)
    for line in [
        "let x = a.unwrap_or(Money::ZERO);",
        "let x = a.unwrap_or( Decimal::ZERO );",
        "let x = a.unwrap_or(MoneyDelta::ZERO);",
        "let x = a.unwrap_or(ForeignAmount::ZERO);",
        "let x = a.unwrap_or(Rate::ZERO);",
        "let x = a.unwrap_or(RatePercent::ZERO);",
        "let x = a.unwrap_or(xc_kernel::money::Money::ZERO);",
    ] {
        assert_eq!(hits(line)[0], 1, "R1 should match: {line}");
    }

    // R2: each type name and integer zero variants
    for line in [
        "let x = a.unwrap_or_else(|_| Money::ZERO);",
        "let x = a.unwrap_or_else(|e| Decimal::ZERO);",
        "let x = a.unwrap_or_else(|_| MoneyDelta::ZERO);",
        "let x = a.unwrap_or_else(|_| ForeignAmount::ZERO);",
        "let x = a.unwrap_or_else(|_| xc_kernel::money::Money::ZERO);",
        "let x = a.unwrap_or_else(|_| 0);",
        "let x = a.unwrap_or_else(|_| 0i64);",
        "let x = a.unwrap_or_else(|_| 0_i64);",
        "let x = a.unwrap_or_else(|_| 0i128);",
        "let x = a.unwrap_or_else(|_| 0_i128);",
    ] {
        assert_eq!(hits(line)[1], 1, "R2 should match: {line}");
    }

    // R3: Decimal::from_str, from_str_exact, and parse::<Decimal>
    for line in [
        r#"let x = Decimal::from_str("1.23").unwrap_or_default();"#,
        r#"let x = Decimal::from_str_exact(s).unwrap_or(Decimal::ONE);"#,
        r#"let x = s.parse::<Decimal>().unwrap_or_default();"#,
        r#"let x = s.parse::< Decimal >().unwrap_or(d);"#,
    ] {
        assert_eq!(hits(line)[2], 1, "R3 should match: {line}");
    }

    // R4: each type × each constructor × each unwrap_or variant
    for ty in ["Money", "ForeignAmount", "Rate", "MoneyDelta"] {
        for method in ["parse", "from_minor", "try_from_minor_i128"] {
            for tail in [
                "unwrap_or_default()",
                "unwrap_or(fallback)",
                "unwrap_or_else(|_| f)",
            ] {
                let line = format!("let x = {ty}::{method}(v).{tail};");
                assert_eq!(hits(&line)[3], 1, "R4 should match: {line}");
            }
        }
    }

    // R5: Decimal::new(0, d), dec!(0), dec!(0.00), Decimal::from(0), and .map_or(<Type>::ZERO,
    for line in [
        "let x = a.unwrap_or(Decimal::new(0, 2));",
        "let x = a.unwrap_or(dec!(0));",
        "let x = a.unwrap_or(dec!(0.00));",
        "let x = a.unwrap_or(Decimal::from(0));",
        "let x = a.map_or(Money::ZERO, |v| v);",
        "let x = a.map_or(Decimal::ZERO, |v| v);",
        "let x = a.map_or(MoneyDelta::ZERO, |v| v);",
    ] {
        assert_eq!(hits(line)[4], 1, "R5 should match: {line}");
    }
}

#[test]
fn the_matcher_ignores_lookalikes() {
    for line in [
        "let x = count.unwrap_or(0);",
        "let x = a.unwrap_or(Money::ONE);",
        "let x = name.unwrap_or_default();",
        "let x = Decimal::from_str(x)?;",
    ] {
        assert_eq!(
            hits(line),
            [0, 0, 0, 0, 0],
            "lookalike must be ignored: {line}"
        );
    }
}

#[test]
fn test_code_and_comments_are_skipped() {
    let fixture = r#"
// let commented = a.unwrap_or(Money::ZERO);
fn prod() {
    let real = a.unwrap_or(Money::ZERO); // let trailing = b.unwrap_or(Money::ZERO);
}

#[cfg(test)]
fn helper_in_test() { let _ = a.unwrap_or(Money::ZERO); }

#[cfg(test)]
mod t {
    #[test]
    fn inside_test_mod() {
        let _ = a.unwrap_or(Money::ZERO);
        let _ = Decimal::from_str("1").unwrap_or_default();
    }
}
"#;
    let total = hits_in_source(fixture);
    assert_eq!(total, [1, 0, 0, 0, 0]);
}

#[test]
fn the_scan_sees_the_tree() {
    let root = api_root();
    let (_, visited) = scan(&root);
    assert!(
        visited.len() >= 200,
        "expected at least 200 scanned files, got {}",
        visited.len()
    );
    assert!(
        visited.contains("crates/xc-kernel/src/money/money.rs"),
        "scanned set must include crates/xc-kernel/src/money/money.rs"
    );
}
