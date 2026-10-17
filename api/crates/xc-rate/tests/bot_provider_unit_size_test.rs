//! Tests for `BotProvider` `unit` / `unit_size` validation (`108-Plaza/xchange#2793`).

use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::rate::entities::rate_source::ReferenceRates;
use xc_rate::domain::reference::reference_provider::ReferenceProvider;
use xc_rate::infrastructure::reference::bot_provider::BotProvider;

fn rates(rows: Vec<Value>) -> ReferenceRates {
    ReferenceRates {
        source: "https://example.com/bot".to_string(),
        fetched_at: "2026-10-09T10:00:00Z".to_string(),
        kind: "BOT".to_string(),
        rows,
    }
}

fn assert_rate_unavailable_mentions(err: RateEngineError, code: &str, raw_fragment: &str) {
    match err {
        RateEngineError::RateUnavailable(msg) => {
            assert!(
                msg.contains(code),
                "expected error message to name currency {code:?}, got: {msg:?}"
            );
            assert!(
                msg.contains(raw_fragment),
                "expected error message to include raw unit {raw_fragment:?}, got: {msg:?}"
            );
        }
        other => panic!("expected RateEngineError::RateUnavailable, got: {other:?}"),
    }
}

// ── R1: unit = "abc" ────────────────────────────────────────────────────────────────────────

#[test]
fn r1_unreadable_string_unit_returns_rate_unavailable_naming_currency() {
    let rows = vec![json!({
        "code": "JPY",
        "date": "2026-10-09",
        "unit": "abc",
        "buyingTransfer": "23.4000",
        "selling": "23.8000"
    })];

    let err = BotProvider
        .parse(&rates(rows))
        .expect_err("unit = \"abc\" must be rejected");
    assert_rate_unavailable_mentions(err, "JPY", "abc");
}

// ── R2: unit = "" ───────────────────────────────────────────────────────────────────────────

#[test]
fn r2_empty_string_unit_returns_rate_unavailable_naming_currency() {
    let rows = vec![json!({
        "code": "JPY",
        "date": "2026-10-09",
        "unit": "",
        "buyingTransfer": "23.4000",
        "selling": "23.8000"
    })];

    let err = BotProvider
        .parse(&rates(rows))
        .expect_err("unit = \"\" must be rejected");
    assert_rate_unavailable_mentions(err, "JPY", "\"\"");
}

// ── R3: unit = 0 (JSON integer) ─────────────────────────────────────────────────────────────

#[test]
fn r3_zero_integer_unit_returns_rate_unavailable_naming_currency() {
    let rows = vec![json!({
        "code": "JPY",
        "date": "2026-10-09",
        "unit": 0,
        "buyingTransfer": "23.4000",
        "selling": "23.8000"
    })];

    let err = BotProvider
        .parse(&rates(rows))
        .expect_err("unit = 0 must be rejected");
    assert_rate_unavailable_mentions(err, "JPY", "0");
}

// ── R4: unit = "-100" ───────────────────────────────────────────────────────────────────────

#[test]
fn r4_negative_unit_returns_rate_unavailable_naming_currency() {
    let rows = vec![json!({
        "code": "JPY",
        "date": "2026-10-09",
        "unit": "-100",
        "buyingTransfer": "23.4000",
        "selling": "23.8000"
    })];

    let err = BotProvider
        .parse(&rates(rows))
        .expect_err("unit = \"-100\" must be rejected");
    assert_rate_unavailable_mentions(err, "JPY", "-100");
}

// ── R5: unit = {} (JSON object) ─────────────────────────────────────────────────────────────

#[test]
fn r5_object_unit_returns_rate_unavailable_naming_currency() {
    let rows = vec![json!({
        "code": "EUR",
        "date": "2026-10-09",
        "unit": {},
        "buyingTransfer": "38.2000",
        "selling": "38.7000"
    })];

    let err = BotProvider
        .parse(&rates(rows))
        .expect_err("unit = {} must be rejected");
    assert_rate_unavailable_mentions(err, "EUR", "{}");
}

// ── K1: unit = 100 (JSON integer) ───────────────────────────────────────────────────────────

#[test]
fn k1_positive_integer_unit_parses_to_decimal() {
    let rows = vec![json!({
        "code": "JPY",
        "date": "2026-10-09",
        "unit": 100,
        "buyingTransfer": "23.4000",
        "selling": "23.8000"
    })];

    let batch = BotProvider.parse(&rates(rows)).expect("unit = 100 parses");
    assert_eq!(batch.lines.len(), 1);
    assert_eq!(batch.lines[0].unit_size, Decimal::from(100));
}

// ── K2: unit = "1" and trimmed strings ──────────────────────────────────────────────────────

#[test]
fn k2_string_and_trimmed_string_unit_parse_to_decimal() {
    let rows = vec![
        json!({
            "code": "USD",
            "date": "2026-10-09",
            "unit": "1",
            "buyingTransfer": "35.1000",
            "selling": "35.5000"
        }),
        json!({
            "code": "JPY",
            "date": "2026-10-09",
            "unit": " 100 ",
            "buyingTransfer": "23.4000",
            "selling": "23.8000"
        }),
    ];

    let batch = BotProvider.parse(&rates(rows)).expect("string units parse");
    assert_eq!(batch.lines[0].unit_size, Decimal::ONE);
    assert_eq!(batch.lines[1].unit_size, Decimal::from(100));
}

// ── K3: unit absent or null -> Decimal::ONE ─────────────────────────────────────────────────

#[test]
fn k3_absent_or_null_unit_defaults_to_one() {
    let rows = vec![
        json!({
            "code": "USD",
            "date": "2026-10-09",
            "buyingTransfer": "35.1000",
            "selling": "35.5000"
        }),
        json!({
            "code": "EUR",
            "date": "2026-10-09",
            "unit": null,
            "buyingTransfer": "38.2000",
            "selling": "38.7000"
        }),
    ];

    let batch = BotProvider
        .parse(&rates(rows))
        .expect("absent or null unit defaults to 1");
    assert_eq!(batch.lines[0].unit_size, Decimal::ONE);
    assert_eq!(batch.lines[1].unit_size, Decimal::ONE);
}

// ── K4: unit = 100.0 (JSON float) -> Decimal::from(100) ────────────────────────────────────

#[test]
fn k4_float_unit_parses_to_decimal() {
    let rows = vec![json!({
        "code": "JPY",
        "date": "2026-10-09",
        "unit": 100.0,
        "buyingTransfer": "23.4000",
        "selling": "23.8000"
    })];

    let batch = BotProvider
        .parse(&rates(rows))
        .expect("unit = 100.0 parses");
    assert_eq!(batch.lines.len(), 1);
    assert_eq!(batch.lines[0].unit_size, Decimal::from(100));
}

// ── K5: unit_size key used instead of unit ──────────────────────────────────────────────────

#[test]
fn k5_unit_size_key_is_read_and_validated_same_as_unit() {
    let ok_rows = vec![
        json!({
            "code": "JPY",
            "date": "2026-10-09",
            "unit_size": 100,
            "buyingTransfer": "23.4000",
            "selling": "23.8000"
        }),
        json!({
            "code": "USD",
            "date": "2026-10-09",
            "unit_size": null,
            "buyingTransfer": "35.1000",
            "selling": "35.5000"
        }),
    ];

    let batch = BotProvider
        .parse(&rates(ok_rows))
        .expect("valid unit_size parses");
    assert_eq!(batch.lines[0].unit_size, Decimal::from(100));
    assert_eq!(batch.lines[1].unit_size, Decimal::ONE);

    let bad_rows = vec![json!({
        "code": "GBP",
        "date": "2026-10-09",
        "unit_size": "bad",
        "buyingTransfer": "45.0000",
        "selling": "45.6000"
    })];
    let err = BotProvider
        .parse(&rates(bad_rows))
        .expect_err("invalid unit_size must be rejected");
    assert_rate_unavailable_mentions(err, "GBP", "bad");
}

// ── §9 Edge cases ───────────────────────────────────────────────────────────────────────────

#[test]
fn batch_with_one_bad_unit_fails_the_whole_parse() {
    let rows = vec![
        json!({
            "code": "USD",
            "date": "2026-10-09",
            "unit": 1,
            "buyingTransfer": "35.1000",
            "selling": "35.5000"
        }),
        json!({
            "code": "JPY",
            "date": "2026-10-09",
            "unit": "abc",
            "buyingTransfer": "23.4000",
            "selling": "23.8000"
        }),
    ];

    let err = BotProvider
        .parse(&rates(rows))
        .expect_err("one bad row must fail the whole batch");
    assert_rate_unavailable_mentions(err, "JPY", "abc");
}

#[test]
fn duplicate_currency_row_with_bad_unit_is_skipped_and_does_not_fail_batch() {
    let rows = vec![
        json!({
            "code": "JPY",
            "date": "2026-10-09",
            "unit": 100,
            "buyingTransfer": "23.4000",
            "selling": "23.8000"
        }),
        json!({
            "code": "JPY",
            "date": "2026-10-09",
            "unit": "abc",
            "buyingTransfer": "99.0000",
            "selling": "99.9000"
        }),
    ];

    let batch = BotProvider
        .parse(&rates(rows))
        .expect("skipped duplicate row with bad unit must not fail the batch");
    assert_eq!(batch.lines.len(), 1);
    assert_eq!(batch.lines[0].unit_size, Decimal::from(100));
}

#[test]
fn row_without_currency_code_and_bad_unit_is_skipped() {
    let rows = vec![
        json!({
            "date": "2026-10-09",
            "unit": "abc",
            "buyingTransfer": "1.0",
            "selling": "2.0"
        }),
        json!({
            "code": "   ",
            "date": "2026-10-09",
            "unit": 0,
            "buyingTransfer": "1.0",
            "selling": "2.0"
        }),
        json!({
            "code": "USD",
            "date": "2026-10-09",
            "unit": 1,
            "buyingTransfer": "35.1000",
            "selling": "35.5000"
        }),
    ];

    let batch = BotProvider
        .parse(&rates(rows))
        .expect("rows without currency code are skipped before unit is checked");
    assert_eq!(batch.lines.len(), 1);
    assert_eq!(batch.lines[0].currency, "USD");
    assert_eq!(batch.lines[0].unit_size, Decimal::ONE);
}
