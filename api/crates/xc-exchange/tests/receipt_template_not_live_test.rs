//! Written before the implementation (RF-6 for the server receipt). Do not edit,
//! delete or weaken.
//!
//! Blueprint §05 (`docs/design/blueprint/rates.md`) forbids any display that claims
//! rates are real time or live. The receipt the server prints
//! (`src/domain/exchange/entities/receipt_template.html`, used by `receipt_html`)
//! carried v2's marketing text "USDT REAL-TIME RATE UPDATE" and
//! "Scan QR Code for / Live Exchange Rates / & Rate Reservation". The web sample
//! dropped the same texts in RF-6 #891; the server receipt uses the same wording:
//!
//! - promo heading: `USDT RATE UPDATE<br>&amp; RATE RESERVATION`
//! - scan copy:     `<b>Scan QR Code for</b><br>Rate Reservation`
//!
//! The text is checked per text node (and per `alt` / `aria-label` / `title`), so a
//! word split from its neighbour by `<br>` is still caught.
//!
//! No database is used here.

use serde_json::Value;

/// The markup `receipt_html` fills (the same file `receipt.rs` includes).
const SERVER_TEMPLATE: &str = include_str!("../src/domain/exchange/entities/receipt_template.html");
/// The markup the byte-for-byte receipt test expects.
const FIXTURE_TEMPLATE: &str = include_str!("fixtures/receipt_template.html");

const PROMO: &str = "<h2 class=\"promo\">USDT RATE UPDATE<br>&amp; RATE RESERVATION</h2>";
const SCAN_COPY: &str =
    "<div class=\"scan-copy\"><b>Scan QR Code for</b><br>Rate Reservation</div>";

/// `(?i)real[\s-]?time|\blive\b|เรียลไทม์|实时`, without a regex dependency.
fn claims_live(text: &str) -> bool {
    let lower = text.to_lowercase();
    if lower.contains("เรียลไทม์") || lower.contains("实时") {
        return true;
    }
    let bytes = lower.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    for (i, _) in lower.match_indices("real") {
        let rest = &lower[i + 4..];
        let rest = rest
            .strip_prefix(|c: char| c.is_whitespace() || c == '-')
            .unwrap_or(rest);
        if rest.starts_with("time") {
            return true;
        }
    }
    for (i, _) in lower.match_indices("live") {
        let before_ok = i == 0 || !is_word(bytes[i - 1]);
        let after = i + 4;
        let after_ok = after >= bytes.len() || !is_word(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// The text a reader meets: every text node outside `<style>`/`<script>`, and the
/// values of `alt`, `aria-label` and `title`.
fn readable_texts(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    let mut skip_until: Option<&str> = None;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            if skip_until.is_none() {
                out.push(rest.to_owned());
            }
            break;
        };
        if skip_until.is_none() && open > 0 {
            out.push(rest[..open].to_owned());
        }
        let tag_and_after = &rest[open..];
        let close = tag_and_after
            .find('>')
            .map_or(tag_and_after.len(), |c| c + 1);
        let tag = &tag_and_after[..close];
        let tag_lower = tag.to_ascii_lowercase();
        if let Some(end) = skip_until {
            if tag_lower.starts_with(end) {
                skip_until = None;
            }
        } else {
            if tag_lower.starts_with("<style") {
                skip_until = Some("</style");
            } else if tag_lower.starts_with("<script") {
                skip_until = Some("</script");
            }
            for attr in ["alt=\"", "aria-label=\"", "title=\""] {
                let mut from = 0;
                while let Some(at) = tag_lower[from..].find(attr) {
                    let start = from + at + attr.len();
                    let boundary =
                        from + at == 0 || tag.as_bytes()[from + at - 1].is_ascii_whitespace();
                    let end = tag[start..].find('"').map_or(tag.len(), |e| start + e);
                    if boundary {
                        out.push(tag[start..end].to_owned());
                    }
                    from = end;
                }
            }
        }
        rest = &tag_and_after[close..];
    }
    out.into_iter()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect()
}

fn live_claims(html: &str) -> Vec<String> {
    readable_texts(html)
        .into_iter()
        .filter(|t| claims_live(t))
        .collect()
}

/// Every receipt v2 recorded in the fixture, as the server template prints it.
fn rendered_receipts() -> Vec<(String, String)> {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/v2_save_form_cases.json"))
        .expect("tests/fixtures/v2_save_form_cases.json parses");
    let cases = fixture["cases"].as_array().expect("cases is an array");
    let mut out = Vec::new();
    for case in cases {
        let Some(receipt) = case["saved"].get("receipt") else {
            continue;
        };
        let part = |key: &str| receipt[key].as_str().unwrap_or_default().to_owned();
        let html = SERVER_TEMPLATE
            .replacen("{{TITLE}}", &part("title"), 1)
            .replacen("{{DEMO_BANNER}}", &part("demoBanner"), 1)
            .replacen("{{CURRENCY}}", &part("currency"), 1)
            .replacen("{{ROWS}}", &part("rows"), 1)
            .replacen("{{TOTAL}}", &part("total"), 1);
        out.push((case["name"].as_str().unwrap_or("?").to_owned(), html));
    }
    out
}

#[test]
fn the_matcher_catches_what_section_05_forbids() {
    for bad in [
        "USDT REAL-TIME RATE UPDATE",
        "real time",
        "Realtime",
        "Live Exchange Rates",
        "rates are LIVE",
        "อัตราเรียลไทม์",
        "实时汇率",
    ] {
        assert!(claims_live(bad), "{bad:?} must be caught");
    }
    for fine in [
        "Rate Reservation",
        "Delivery",
        "deliver",
        "olive",
        "lived",
        "Realm times",
    ] {
        assert!(!claims_live(fine), "{fine:?} must not be caught");
    }
    assert_eq!(
        readable_texts("<b>Scan QR Code for</b><br>Live Exchange Rates"),
        vec!["Scan QR Code for", "Live Exchange Rates"],
        "text nodes are split at tags"
    );
}

#[test]
fn the_server_receipt_template_says_nothing_is_real_time_or_live() {
    let found = live_claims(SERVER_TEMPLATE);
    assert!(
        found.is_empty(),
        "receipt_template.html claims live rates in: {found:?}"
    );
}

#[test]
fn the_fixture_template_says_nothing_is_real_time_or_live() {
    let found = live_claims(FIXTURE_TEMPLATE);
    assert!(
        found.is_empty(),
        "tests/fixtures/receipt_template.html claims live rates in: {found:?}"
    );
}

#[test]
fn every_printed_receipt_says_nothing_is_real_time_or_live() {
    let receipts = rendered_receipts();
    assert!(!receipts.is_empty(), "the fixture holds saved receipts");
    let wrong: Vec<String> = receipts
        .iter()
        .filter_map(|(name, html)| {
            let found = live_claims(html);
            (!found.is_empty()).then(|| format!("'{name}': {found:?}"))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} receipt(s) claim live rates:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn the_server_receipt_uses_the_rf6_wording() {
    assert_eq!(
        SERVER_TEMPLATE.matches(PROMO).count(),
        1,
        "the promo heading reads {PROMO}"
    );
    assert_eq!(
        SERVER_TEMPLATE.matches(SCAN_COPY).count(),
        1,
        "the scan copy reads {SCAN_COPY}"
    );
}

#[test]
fn the_server_template_is_the_fixture_template_byte_for_byte() {
    assert!(
        SERVER_TEMPLATE == FIXTURE_TEMPLATE,
        "src/domain/exchange/entities/receipt_template.html must equal \
         tests/fixtures/receipt_template.html (lengths {} and {})",
        SERVER_TEMPLATE.len(),
        FIXTURE_TEMPLATE.len()
    );
}
