//! Up-front test for R3 (#564 issuecomment-6542427118, -6542441849): the source guard on readers of
//! `published_rate_lines`, made strict. It replaces the guard half of `rate_feed_first_tier_views_test.rs`
//! (F11, `rate-feed-connector.md` §9.4 "Guard"), which accepted any read whose SQL mentions `value_tier`, so a
//! `value_tier = 'ALL'` filter passed it, and which allowed `xc-rate`'s `published_rates.rs` as a whole file. Both
//! holes let readers through that answer nothing on a tier-only (MANUAL, `pricing-tiers.md` §15.3) publication:
//! `PgPublishedRates::line_in` (rate alerts) and `ledger_postings.rs` `published_buy_unit_cost`.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken; add
//! tests in a file of your own. Needs no database.
//!
//! The rule, per read of `published_rate_lines` under `api/src` and `api/crates/*/src` (a line naming the table,
//! with 8 lines either side as its SQL window; comments and `INSERT INTO` skipped):
//! 1. names `tier_lower_exclusive_thb`: picks the first tier (R-10) → passes;
//! 2. otherwise filters `value_tier = 'ALL'`: reads only an untiered line, nothing on a tier-only publication →
//!    **refused**, with no exception;
//! 3. otherwise names `value_tier`: reads the tiers and picks one itself → passes;
//! 4. otherwise reads one line by id (`id = $n`) → passes;
//! 5. otherwise refused, unless its file is in `ALLOWED` (writers, and readers that need any tier).

use std::path::{Path, PathBuf};

/// Files whose reads may name no tier. Each entry says why. No reader of prices is here.
const ALLOWED: &[&str] = &[
    // write publications (their reads copy a whole publication)
    "crates/xc-rate/src/application/publication/publish_service.rs",
    "crates/xc-rate/src/application/publication/feed_publisher.rs",
    // asks whether a group is priced at all, any tier
    "crates/xc-rate/src/application/currency/denomination_group_service.rs",
    // DISTINCT currency codes of a publication: any tier gives the same set
    "src/presentation/http/v3/views/admin_cash.rs",
    // a doc comment naming the table
    "crates/xc-kiosk/src/domain/kiosk/value_objects/counter_facts.rs",
];

fn api_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn all_tier_filter(window: &str) -> bool {
    let squeezed: String = window.split_whitespace().collect::<Vec<_>>().join(" ");
    let squeezed = squeezed.replace("= '", "='").replace(" ='", "='");
    squeezed.contains("value_tier='ALL'")
}

fn by_id(window: &str) -> bool {
    // `id = $n` on its own column: `l.id = $1`, `WHERE id = $1`; not `publication_id = $1`.
    let squeezed: String = window.split_whitespace().collect::<Vec<_>>().join(" ");
    let squeezed = squeezed.replace("id=$", "id = $");
    squeezed.match_indices("id = $").any(|(at, _)| {
        !squeezed[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Why a read with this SQL window is refused, or `None` when it passes (rules 1–4).
fn refusal(window: &str) -> Option<&'static str> {
    if window.contains("tier_lower_exclusive_thb") {
        None
    } else if all_tier_filter(window) {
        Some("filters value_tier = 'ALL': answers nothing on a tier-only publication")
    } else if window.contains("value_tier") || by_id(window) {
        None
    } else {
        Some("names no tier")
    }
}

/// `(file relative to api/, line, why)` for every refused read in `text`.
fn refused_reads(rel: &str, text: &str) -> Vec<(String, usize, &'static str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") || !line.contains("published_rate_lines") {
            continue;
        }
        if line.contains("INSERT INTO published_rate_lines") {
            continue;
        }
        let from = i.saturating_sub(8);
        let to = (i + 9).min(lines.len());
        let window = lines[from..to].join("\n");
        if let Some(why) = refusal(&window) {
            if why == "names no tier" && ALLOWED.contains(&rel) {
                continue;
            }
            found.push((rel.to_owned(), i + 1, why));
        }
    }
    found
}

fn scan() -> Vec<(String, usize, &'static str)> {
    let root = api_dir();
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let crates = std::fs::read_dir(root.join("crates")).expect("api/crates exists");
    for c in crates.flatten() {
        rust_files(&c.path().join("src"), &mut files);
    }
    assert!(
        files.len() > 100,
        "the scan reaches api/src and api/crates/*/src"
    );
    let mut found = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&root)
            .expect("under api/")
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&file).expect("readable source");
        found.extend(refused_reads(&rel, &text));
    }
    found.sort();
    found
}

#[test]
fn every_reader_of_published_rate_lines_reads_a_tier_or_one_line_by_id() {
    let found = scan();
    assert!(
        found.is_empty(),
        "these read published_rate_lines with no tier rule (R-10: `tier_lower_exclusive_thb IS NULL`, or \
         `PublishedRates::line_for`; design rate-feed-connector.md §9.1, §9.4): {found:#?}"
    );
}

#[test]
fn a_planted_all_tier_reader_is_refused_even_in_an_allowed_file() {
    let planted = "let row = sqlx::query(\n\
        \"SELECT l.buy_rate FROM branches b \\\n\
         JOIN published_rate_lines l ON l.publication_id = b.current_publication_id \\\n\
         WHERE b.id = $1 AND l.denomination_group = 'ALL' AND l.value_tier = 'ALL'\",\n\
        );";
    let refused = refused_reads(
        "crates/xc-rate/src/application/publication/publish_service.rs",
        planted,
    );
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(
        refused[0].2,
        "filters value_tier = 'ALL': answers nothing on a tier-only publication"
    );
    let spaced = planted.replace("value_tier = 'ALL'", "value_tier='ALL'");
    assert_eq!(
        refused_reads("src/x.rs", &spaced).len(),
        1,
        "no spacing escapes it"
    );
}

#[test]
fn the_rules_that_pass() {
    // R-10 first tier, also with a preference for the ALL line (R2's shape).
    let first_tier = "JOIN published_rate_lines l ON l.publication_id = p.id \
        WHERE l.denomination_group = 'ALL' AND l.tier_lower_exclusive_thb IS NULL \
        ORDER BY (l.value_tier = 'ALL') DESC LIMIT 1";
    assert!(refused_reads("src/x.rs", first_tier).is_empty());
    // Reads the tiers and picks one itself (line_for).
    let picks = "SELECT l.id, l.value_tier, l.base_buy_rate FROM published_rate_lines l WHERE l.publication_id = $1";
    assert!(refused_reads("src/x.rs", picks).is_empty());
    // One line by id.
    let one = "SELECT source_snapshot FROM published_rate_lines WHERE id = $1";
    assert!(refused_reads("src/x.rs", one).is_empty());
    // No tier, not by id, outside ALLOWED.
    let list = "SELECT currency_code, buy_rate FROM published_rate_lines WHERE publication_id = $1 AND denomination_group = 'ALL'";
    assert_eq!(refused_reads("src/x.rs", list).len(), 1);
}
