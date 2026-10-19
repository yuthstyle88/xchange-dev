//! G-1 guard (#2054, design `docs/design/blueprint/bounded-inputs.md` §2 B-4, B-5, §5 `:107-114`) — the provided
//! tests, written before the implementation by claude/studio-claude-2 on the order of the xchange BE lane controller
//! claude/56a74fcd. Do not edit, delete or weaken; the scanner and the allowlist live in `bounded_inputs_scan/mod.rs`.
//!
//! A presentation file (`api/src/presentation/**`) must not parse a decimal outside the bounded core:
//! no `Decimal::from_str(`, `ForeignAmount::from_decimal(` or `UnitSize::from_decimal(`, and no `Decimal`,
//! `Option<Decimal>` or `Vec<Decimal>` field in a `Deserialize` struct. Comments and string literals never count
//! (#2054 issuecomment-6231374089, point 2). Anything else passes only through the allowlist, one entry per file
//! with a reason; a stale entry fails too.

mod bounded_inputs_scan;

use bounded_inputs_scan::{ALLOWLIST, Report, load_presentation_files, scan_sources};

const CMD: &str = "src/presentation/http/v3/commands/fixture.rs";
const DTO: &str = "src/presentation/dto/v3/fixture.rs";

fn scan_one(path: &str, src: &str) -> Report {
    scan_sources(&[(path, src)], &[])
}

fn flagged(path: &str, src: &str, what: &str) {
    let r = scan_one(path, src);
    assert!(
        r.findings
            .iter()
            .any(|f| f.file == path && f.what.contains(what)),
        "{what} in {path} must be flagged, got {r:?}\n--- source ---\n{src}"
    );
}

fn clean(path: &str, src: &str) {
    let r = scan_one(path, src);
    assert!(r.is_ok(), "must pass, got {r:?}\n--- source ---\n{src}");
}

// ================================================================ the real tree

#[test]
fn presentation_inputs_are_bounded_or_allowlisted() {
    let files = load_presentation_files();
    assert!(
        files.len() > 100,
        "the scan must see the presentation tree (got {} files)",
        files.len()
    );
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, s)| (p.as_str(), s.as_str()))
        .collect();
    let report = scan_sources(&borrowed, ALLOWLIST);
    assert!(report.is_ok(), "{report:#?}");
}

// ================================================================ fixtures: parse calls

#[test]
fn f01_decimal_from_str_in_a_command_is_flagged() {
    flagged(
        CMD,
        "fn run(t: &str) { let q = Decimal::from_str(t).unwrap(); }\n",
        "Decimal::from_str(",
    );
}

#[test]
fn f02_foreign_amount_from_decimal_is_flagged() {
    flagged(
        CMD,
        "fn run(d: Decimal) { let a = ForeignAmount::from_decimal(d); }\n",
        "ForeignAmount::from_decimal(",
    );
}

#[test]
fn f03_unit_size_from_decimal_is_flagged() {
    flagged(
        DTO,
        "fn f(d: Decimal) { let u = UnitSize::from_decimal(d); }\n",
        "UnitSize::from_decimal(",
    );
}

#[test]
fn f04_a_fully_qualified_call_is_flagged_too() {
    flagged(
        CMD,
        "fn run(t: &str) { rust_decimal::Decimal::from_str(t).ok(); }\n",
        "Decimal::from_str(",
    );
}

#[test]
fn f05_the_same_call_outside_presentation_is_not_scanned() {
    clean(
        "src/application/fixture.rs",
        "fn run(t: &str) { Decimal::from_str(t).ok(); }\n",
    );
    clean(
        "crates/xc-kernel/src/money/x.rs",
        "fn run(t: &str) { Decimal::from_str(t).ok(); }\n",
    );
}

// ================================================================ fixtures: comments and string literals

#[test]
fn f06_a_line_comment_does_not_count() {
    clean(CMD, "// Decimal::from_str(t) was here\nfn run() {}\n");
    clean(CMD, "fn run() {} // ForeignAmount::from_decimal(d)\n");
}

#[test]
fn f07_a_block_comment_does_not_count() {
    clean(
        CMD,
        "/* Decimal::from_str(t)\n   UnitSize::from_decimal(d) */\nfn run() {}\n",
    );
    clean(CMD, "fn run() { /* ForeignAmount::from_decimal(d) */ }\n");
}

#[test]
fn f08_a_string_literal_does_not_count() {
    clean(CMD, "fn run() -> &'static str { \"Decimal::from_str(\" }\n");
    clean(
        CMD,
        "fn run() -> &'static str { r#\"ForeignAmount::from_decimal(\"#  }\n",
    );
}

#[test]
fn f09_code_after_a_comment_marker_inside_a_string_still_counts() {
    flagged(
        CMD,
        "fn run(t: &str) { let u = \"http://x\"; let q = Decimal::from_str(t); }\n",
        "Decimal::from_str(",
    );
}

// ================================================================ fixtures: Deserialize fields (B-4)

#[test]
fn f10_a_decimal_field_in_a_deserialize_struct_is_flagged() {
    flagged(
        DTO,
        "#[derive(Deserialize)]\nstruct In { quantity: Decimal }\n",
        "quantity",
    );
}

#[test]
fn f11_option_and_vec_of_decimal_are_flagged() {
    flagged(
        DTO,
        "#[derive(Deserialize)]\npub struct In {\n    pub limit: Option<Decimal>,\n}\n",
        "limit",
    );
    flagged(
        DTO,
        "#[derive(Deserialize)]\npub struct In {\n    pub floors: Vec<Decimal>,\n}\n",
        "floors",
    );
}

#[test]
fn f12_a_multi_line_derive_and_a_path_type_are_flagged() {
    let src = "#[derive(\n    Debug,\n    Clone,\n    serde::Deserialize,\n)]\n#[serde(rename_all = \"camelCase\")]\n\
               pub struct In {\n    pub name: String,\n    pub buy: rust_decimal::Decimal,\n}\n";
    flagged(CMD, src, "buy");
}

#[test]
fn f13_serialize_only_and_string_fields_pass() {
    clean(
        DTO,
        "#[derive(Serialize)]\npub struct Out { pub total: Decimal }\n",
    );
    clean(
        DTO,
        "#[derive(Deserialize)]\npub struct In { pub quantity: String, pub note: Option<String> }\n",
    );
}

#[test]
fn f14_a_commented_out_decimal_field_does_not_count() {
    clean(
        DTO,
        "#[derive(Deserialize)]\npub struct In {\n    pub quantity: String,\n    // pub old: Decimal,\n}\n",
    );
}

// ================================================================ fixtures: the allowlist

#[test]
fn f15_an_allowlisted_file_with_a_hit_passes() {
    let r = scan_sources(
        &[(CMD, "fn run(t: &str) { Decimal::from_str(t).ok(); }\n")],
        &[(CMD, "#9999 removes it")],
    );
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn f16_a_stale_allowlist_entry_fails() {
    let r = scan_sources(&[(CMD, "fn run() {}\n")], &[(CMD, "#9999 removes it")]);
    assert_eq!(r.stale_allowlist, vec![CMD.to_owned()], "{r:?}");
    assert!(!r.is_ok());
}

#[test]
fn f17_an_allowlist_entry_without_a_reason_fails() {
    let r = scan_sources(
        &[(CMD, "fn run(t: &str) { Decimal::from_str(t).ok(); }\n")],
        &[(CMD, "  ")],
    );
    assert_eq!(r.empty_reasons, vec![CMD.to_owned()], "{r:?}");
    assert!(!r.is_ok());
}

#[test]
fn f18_the_allowlist_never_hides_another_file() {
    let r = scan_sources(
        &[
            (CMD, "fn run(t: &str) { Decimal::from_str(t).ok(); }\n"),
            (DTO, "fn f(t: &str) { Decimal::from_str(t).ok(); }\n"),
        ],
        &[(CMD, "#9999 removes it")],
    );
    assert!(r.findings.iter().any(|f| f.file == DTO), "{r:?}");
    assert!(!r.findings.iter().any(|f| f.file == CMD), "{r:?}");
}
