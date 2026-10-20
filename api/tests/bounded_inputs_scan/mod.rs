//! The scanner behind `bounded_inputs_guard_test.rs` (G-1, #2054; design `docs/design/blueprint/bounded-inputs.md`
//! §5 `:107-114`). It lives in its own module so the provided guard tests stay untouched while the scanner and the
//! allowlist change.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One hit: a presentation file parses a decimal outside the bounded core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The path as given to [`scan_sources`], e.g. `src/presentation/http/v3/commands/quotes.rs`.
    pub file: String,
    /// What matched: `Decimal::from_str(`, `ForeignAmount::from_decimal(`, `UnitSize::from_decimal(`, or
    /// `Deserialize field <name>: <type>`.
    pub what: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Hits in files that are not on the allowlist.
    pub findings: Vec<Finding>,
    /// Allowlisted files with no hit any more.
    pub stale_allowlist: Vec<String>,
    /// Allowlist entries whose reason is empty or only whitespace.
    pub empty_reasons: Vec<String>,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.findings.is_empty() && self.stale_allowlist.is_empty() && self.empty_reasons.is_empty()
    }
}

/// `(path, reason)`. A reason names the open #2054 R-ticket that removes the hit, or says why the file reads no
/// client input there. The list only shrinks.
pub const ALLOWLIST: &[(&str, &str)] = &[
    (
        "src/presentation/dto/v3/my_membership.rs",
        "Loyalty service response DTO, not client input",
    ),
    (
        "src/presentation/dto/v3/planner.rs",
        "budgetThb > 0 pre-check via Decimal::from_str before Money::parse; client input not covered by #2976, which bounded quantity only",
    ),
    (
        "src/presentation/dto/v3/transaction_row.rs",
        "stored transaction row read back from database, not client input",
    ),
    (
        "src/presentation/http/v3/commands/bookings/create.rs",
        "BookingCreateOutput.quantity is the command's response DTO, not client input; the input is bounded by #2983",
    ),
    (
        "src/presentation/http/v3/commands/campaign_simulate.rs",
        "parses the stored currencies.unit_size read back from the database, not client input",
    ),
    (
        "src/presentation/http/v3/commands/cash.rs",
        "TransferPolicySave checks non-negative and currency minor_unit scale via Decimal::from_str after Money::parse bounds the input (#2116)",
    ),
    (
        "src/presentation/http/v3/commands/margin_guard_set.rs",
        "locally bounded 0..1 margin guard (O-G1-2)",
    ),
    (
        "src/presentation/http/v3/commands/quotes.rs",
        "parses the stored currencies.unit_size read back from the database, not client input",
    ),
    (
        "src/presentation/http/v3/desk/config.rs",
        "stored currency unit_size read back from database (O-G1-2)",
    ),
    (
        "src/presentation/http/v3/views/admin_cash.rs",
        "admin cash view read back from database, plus Decimal::from_str literals in its #[cfg(test)] module; not client input",
    ),
    (
        "src/presentation/http/v3/views/admin_reports.rs",
        "admin reports view read back from database, not client input",
    ),
    (
        "src/presentation/http/v3/views/kiosk_currencies.rs",
        "parses buy/sell values of the bundled KBank reference file (include_str! kiosk_currencies.kbank.json); not client input",
    ),
    (
        "src/presentation/http/v3/views/rates_reference.rs",
        "parses rates from the RateSource reference-rate feed response; not client input",
    ),
    (
        "src/presentation/http/v3/views/segment_preview.rs",
        "segment preview view read back from database, not client input",
    ),
    (
        "src/presentation/pages.rs",
        "v2 compat stored decimal formatting and tests (O-G1-2)",
    ),
];

/// Scans presentation sources for unbounded decimal parses or deserialization.
pub fn scan_sources(files: &[(&str, &str)], allowlist: &[(&str, &str)]) -> Report {
    let mut findings = Vec::new();
    let mut stale_allowlist = Vec::new();
    let mut empty_reasons = Vec::new();

    for (allowed_path, reason) in allowlist {
        if reason.trim().is_empty() {
            empty_reasons.push((*allowed_path).to_string());
        }
    }

    let mut hits_per_file: HashMap<&str, Vec<Finding>> = HashMap::new();
    for (path, source) in files {
        let file_findings = scan_single_file(path, source);
        if !file_findings.is_empty() {
            hits_per_file.insert(path, file_findings);
        }
    }

    for (path, file_findings) in &hits_per_file {
        let is_allowlisted = allowlist
            .iter()
            .any(|(allowed_path, _)| allowed_path == path);
        if !is_allowlisted {
            findings.extend(file_findings.clone());
        }
    }

    for (allowed_path, _) in allowlist {
        let has_hits = hits_per_file.contains_key(allowed_path);
        if !has_hits {
            stale_allowlist.push((*allowed_path).to_string());
        }
    }

    findings.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.what.cmp(&b.what)));
    stale_allowlist.sort();
    empty_reasons.sort();

    Report {
        findings,
        stale_allowlist,
        empty_reasons,
    }
}

fn is_presentation(path: &str) -> bool {
    path.starts_with("src/presentation/")
        || path.starts_with("api/src/presentation/")
        || path.starts_with("presentation/")
}

fn scan_single_file(path: &str, source: &str) -> Vec<Finding> {
    if !is_presentation(path) {
        return Vec::new();
    }

    let stripped = strip_comments_and_strings(source);
    let mut findings = Vec::new();

    // 1. Direct call patterns
    find_call_matches(
        &stripped,
        "Decimal::from_str",
        "Decimal::from_str(",
        &mut findings,
        path,
    );
    find_call_matches(
        &stripped,
        "ForeignAmount::from_decimal",
        "ForeignAmount::from_decimal(",
        &mut findings,
        path,
    );
    find_call_matches(
        &stripped,
        "UnitSize::from_decimal",
        "UnitSize::from_decimal(",
        &mut findings,
        path,
    );

    // 2. Deserialize structs with decimal fields
    let tokens = tokenize(&stripped);
    scan_struct_tokens(&tokens, path, &mut findings);

    findings
}

fn find_call_matches(
    stripped: &str,
    target: &str,
    what_name: &str,
    findings: &mut Vec<Finding>,
    file: &str,
) {
    let mut search_from = 0;
    while let Some(pos) = stripped[search_from..].find(target) {
        let abs_pos = search_from + pos;
        search_from = abs_pos + target.len();

        // Check character before: must not be alphanumeric or '_'
        if abs_pos > 0 {
            let prev_ch = stripped[..abs_pos].chars().next_back().unwrap();
            if prev_ch.is_ascii_alphanumeric() || prev_ch == '_' {
                continue;
            }
        }

        // Check character after: optional whitespace then '('
        let rest = &stripped[abs_pos + target.len()..];
        let trimmed_rest = rest.trim_start();
        if trimmed_rest.starts_with('(') {
            findings.push(Finding {
                file: file.to_string(),
                what: what_name.to_string(),
            });
        }
    }
}

fn strip_comments_and_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;

    while i < len {
        // 1. Line comment: `//` to end of line
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // 2. Block comment: `/* ... */` (nested supported)
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1usize;
            while i < len && depth > 0 {
                if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if bytes[i] == b'\n' {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }

        // 3. Raw string literal: `r"..."` or `r#"..."#`
        if bytes[i] == b'r'
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
        {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < len && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < len && bytes[j] == b'"' {
                j += 1;
                let mut found_end = false;
                while j < len {
                    if bytes[j] == b'"'
                        && j + 1 + hashes <= len
                        && bytes[j + 1..j + 1 + hashes].iter().all(|&b| b == b'#')
                    {
                        j += 1 + hashes;
                        found_end = true;
                        break;
                    }
                    if bytes[j] == b'\n' {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                    j += 1;
                }
                if found_end {
                    out.push(' ');
                    i = j;
                    continue;
                }
            }
        }

        // 4. Standard string literal: `"..."`
        if bytes[i] == b'"' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 1;
                    if i < len && bytes[i] == b'\n' {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                    if i < len {
                        i += 1;
                    }
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    if bytes[i] == b'\n' {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }

        // 5. Character literal: `'x'` or `'\n'` or `'\''` or `'\\'`
        if bytes[i] == b'\'' {
            if i + 3 < len && bytes[i + 1] == b'\\' && bytes[i + 3] == b'\'' {
                out.push_str("    ");
                i += 4;
                continue;
            }
            if i + 2 < len && bytes[i + 1] != b'\'' && bytes[i + 2] == b'\'' {
                out.push_str("   ");
                i += 3;
                continue;
            }
        }

        let ch = source[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    text: String,
}

fn tokenize(s: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        if c.is_ascii_alphanumeric() || c == '_' {
            let start = i;
            while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            tokens.push(Token { text: word });
            continue;
        }

        if c == ':' && i + 1 < len && chars[i + 1] == ':' {
            tokens.push(Token {
                text: "::".to_string(),
            });
            i += 2;
            continue;
        }

        tokens.push(Token {
            text: c.to_string(),
        });
        i += 1;
    }

    tokens
}

fn scan_struct_tokens(tokens: &[Token], file: &str, findings: &mut Vec<Finding>) {
    let mut i = 0;
    let mut struct_has_deserialize = false;

    while i < tokens.len() {
        // Check for attribute: #[ ... ]
        if tokens[i].text == "#" && i + 1 < tokens.len() && tokens[i + 1].text == "[" {
            let attr_start = i;
            let mut depth = 0;
            let mut attr_end = i;
            while attr_end < tokens.len() {
                if tokens[attr_end].text == "[" {
                    depth += 1;
                } else if tokens[attr_end].text == "]" {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                attr_end += 1;
            }

            // Look for derive(...) inside tokens[attr_start..=attr_end]
            let mut j = attr_start;
            while j < attr_end && j < tokens.len() {
                if tokens[j].text == "derive" && j + 1 < tokens.len() && tokens[j + 1].text == "(" {
                    let mut d_depth = 0;
                    let mut k = j + 1;
                    while k <= attr_end && k < tokens.len() {
                        if tokens[k].text == "(" {
                            d_depth += 1;
                        } else if tokens[k].text == ")" {
                            d_depth -= 1;
                            if d_depth == 0 {
                                break;
                            }
                        } else if tokens[k].text == "Deserialize" {
                            struct_has_deserialize = true;
                        }
                        k += 1;
                    }
                }
                j += 1;
            }
            i = attr_end + 1;
            continue;
        }

        // Skip visibility: pub, pub(crate), etc.
        if tokens[i].text == "pub" {
            i += 1;
            if i < tokens.len() && tokens[i].text == "(" {
                while i < tokens.len() && tokens[i].text != ")" {
                    i += 1;
                }
                if i < tokens.len() {
                    i += 1;
                }
            }
            continue;
        }

        // Struct keyword
        if tokens[i].text == "struct" {
            let is_target = struct_has_deserialize;
            struct_has_deserialize = false;

            i += 1;
            // Struct name
            if i < tokens.len() {
                i += 1;
            }
            // Skip generics `<...>`
            if i < tokens.len() && tokens[i].text == "<" {
                let mut g_depth = 0;
                while i < tokens.len() {
                    if tokens[i].text == "<" {
                        g_depth += 1;
                    } else if tokens[i].text == ">" {
                        g_depth -= 1;
                        if g_depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
            }
            // Skip where clause until '{' or ';'
            while i < tokens.len() && tokens[i].text != "{" && tokens[i].text != ";" {
                i += 1;
            }

            if i < tokens.len() && tokens[i].text == "{" {
                let mut brace_depth = 1;
                i += 1;
                let mut body_tokens = Vec::new();
                while i < tokens.len() && brace_depth > 0 {
                    if tokens[i].text == "{" {
                        brace_depth += 1;
                        body_tokens.push(tokens[i].clone());
                    } else if tokens[i].text == "}" {
                        brace_depth -= 1;
                        if brace_depth > 0 {
                            body_tokens.push(tokens[i].clone());
                        }
                    } else {
                        body_tokens.push(tokens[i].clone());
                    }
                    i += 1;
                }

                if is_target {
                    scan_struct_fields(&body_tokens, file, findings);
                }
            }
            continue;
        }

        // Reset if other item keyword encountered
        if matches!(
            tokens[i].text.as_str(),
            "fn" | "enum" | "type" | "const" | "static" | "impl" | "trait" | "mod" | "use"
        ) {
            struct_has_deserialize = false;
        }

        i += 1;
    }
}

fn scan_struct_fields(tokens: &[Token], file: &str, findings: &mut Vec<Finding>) {
    let mut i = 0;
    while i < tokens.len() {
        // Skip field attributes: #[ ... ]
        if tokens[i].text == "#" && i + 1 < tokens.len() && tokens[i + 1].text == "[" {
            let mut depth = 0;
            while i < tokens.len() {
                if tokens[i].text == "[" {
                    depth += 1;
                } else if tokens[i].text == "]" {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            continue;
        }

        // Skip visibility: pub, pub(crate), etc.
        if tokens[i].text == "pub" {
            i += 1;
            if i < tokens.len() && tokens[i].text == "(" {
                while i < tokens.len() && tokens[i].text != ")" {
                    i += 1;
                }
                if i < tokens.len() {
                    i += 1;
                }
            }
            continue;
        }

        // Field name followed by ':'
        if i + 1 < tokens.len() && tokens[i + 1].text == ":" {
            let field_name = tokens[i].text.clone();
            i += 2; // skip field_name and ':'

            let mut angle_depth = 0usize;
            let mut paren_depth = 0usize;
            let mut type_tokens = Vec::new();

            while i < tokens.len() {
                let t = &tokens[i].text;
                if t == "<" {
                    angle_depth += 1;
                    type_tokens.push(t.clone());
                } else if t == ">" {
                    angle_depth = angle_depth.saturating_sub(1);
                    type_tokens.push(t.clone());
                } else if t == "(" {
                    paren_depth += 1;
                    type_tokens.push(t.clone());
                } else if t == ")" {
                    paren_depth = paren_depth.saturating_sub(1);
                    type_tokens.push(t.clone());
                } else if t == "," && angle_depth == 0 && paren_depth == 0 {
                    i += 1; // skip ','
                    break;
                } else {
                    type_tokens.push(t.clone());
                }
                i += 1;
            }

            let type_str = type_tokens.join("");
            if is_decimal_type(&type_str) {
                findings.push(Finding {
                    file: file.to_string(),
                    what: format!("Deserialize field {field_name}: {type_str}"),
                });
            }
            continue;
        }

        i += 1;
    }
}

fn is_decimal_type(ty: &str) -> bool {
    let clean: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
    if clean == "Decimal" || clean == "rust_decimal::Decimal" {
        return true;
    }
    if let Some(inner) = clean
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return is_decimal_type(inner);
    }
    if let Some(inner) = clean.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        return is_decimal_type(inner);
    }
    false
}

/// Loads every `.rs` file under `api/src/presentation/**` as `(path relative to api/, contents)`.
pub fn load_presentation_files() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let presentation_dir = root.join("src/presentation");
    let mut paths = Vec::new();
    collect_rs_files(&presentation_dir, &mut paths);
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            (rel, content)
        })
        .collect()
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}
