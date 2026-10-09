//! Ticket #596 (AP-1) — `Redactor`, the core's rule that no secret or full PII reaches a
//! stored payload or an audit row. Written before the implementation by the
//! ticket author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §5.5 (Redaction; Blueprint §07 "no
//! secret or full PII in any log"), §7 (`Redactor` in `xc-approval
//! domain/redactor.rs`), §9 (AP-1: "`Redactor` masks a passport field").
//!
//! Pure: no database.
//!
//! Signatures this test assumes (the design names the type and its rule, not
//! the function):
//! - `xc_approval::domain::redactor::Redactor::redact(value: &serde_json::Value)
//!   -> serde_json::Value`, an associated function: the deny list is the core's,
//!   so there is nothing to configure.
//! - A masked value is a string that keeps the original's last 4 characters and
//!   none of the characters before them; its exact filler (`*`, `•`, a prefix)
//!   is the implementer's.
//! - A dropped value leaves nothing of itself in the output: the key may be
//!   removed or its value replaced, but no character run of the secret stays.
//! - Keys are matched as they are written in this crate's payloads: `passport`,
//!   `passportNo` (the name v3 already uses), `phone` and `idNumber` are
//!   masked; `secret`, `token`, `password` and `otp` are dropped. A key named
//!   `id` is deliberately not used here: whether a bare `id` is an "id number"
//!   is not decided by the design.

use serde_json::{Value, json};
use xc_approval::domain::redactor::Redactor;

/// The string at `path` in `value`, failing the test when it is absent or not a string.
fn string_at<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    let mut here = value;
    for step in path {
        here = here
            .get(step)
            .unwrap_or_else(|| panic!("{path:?}: '{step}' is missing in {value}"));
    }
    here.as_str()
        .unwrap_or_else(|| panic!("{path:?} is not a string in {value}"))
}

fn assert_masked(out: &Value, path: &[&str], original: &str) {
    let masked = string_at(out, path);
    let last4: String = original
        .chars()
        .skip(original.chars().count() - 4)
        .collect();
    let head: String = original
        .chars()
        .take(original.chars().count() - 4)
        .collect();
    assert!(
        masked.ends_with(&last4),
        "{path:?}: '{masked}' must keep the last 4 characters '{last4}'"
    );
    assert!(
        !masked.contains(&head),
        "{path:?}: '{masked}' still holds '{head}', the part before the last 4"
    );
    assert_ne!(masked, original, "{path:?} was not masked");
}

fn assert_gone(out: &Value, secret: &str) {
    let text = out.to_string();
    assert!(
        !text.contains(secret),
        "the secret '{secret}' is still in {text}"
    );
    let last4: String = secret.chars().skip(secret.chars().count() - 4).collect();
    assert!(
        !text.contains(&last4),
        "'{last4}' (the end of a secret) is in {text}: a secret is dropped, not masked"
    );
}

#[test]
fn a_passport_field_is_masked_to_its_last_four_characters() {
    let out = Redactor::redact(&json!({ "passport": "AA1234567" }));
    assert_masked(&out, &["passport"], "AA1234567");
}

#[test]
fn passport_number_phone_and_id_number_fields_are_masked() {
    let out = Redactor::redact(&json!({
        "passportNo": "XK9988776655",
        "phone": "0812345678",
        "idNumber": "1103700012345",
    }));
    assert_masked(&out, &["passportNo"], "XK9988776655");
    assert_masked(&out, &["phone"], "0812345678");
    assert_masked(&out, &["idNumber"], "1103700012345");
}

#[test]
fn secret_token_password_and_otp_values_are_dropped() {
    let out = Redactor::redact(&json!({
        "secret": "s3cr3t-QWER",
        "token": "tok-ASDF1",
        "password": "hunter2-ZXCV",
        "otp": "918273",
        "currency": "USD",
    }));
    assert_gone(&out, "s3cr3t-QWER");
    assert_gone(&out, "tok-ASDF1");
    assert_gone(&out, "hunter2-ZXCV");
    assert_gone(&out, "918273");
    assert_eq!(string_at(&out, &["currency"]), "USD");
}

#[test]
fn nested_objects_and_arrays_are_redacted_too() {
    let out = Redactor::redact(&json!({
        "customer": {
            "passportNo": "XK9988776655",
            "contact": { "phone": "0812345678", "otp": "564738" },
        },
        "lines": [
            { "currency": "JPY", "token": "tok-LKJH9" },
            { "currency": "USD", "idNumber": "1103700012345" },
        ],
    }));
    assert_masked(&out, &["customer", "passportNo"], "XK9988776655");
    assert_masked(&out, &["customer", "contact", "phone"], "0812345678");
    assert_gone(&out, "564738");
    assert_gone(&out, "tok-LKJH9");
    let lines = out["lines"].as_array().expect("lines stays an array");
    assert_eq!(lines.len(), 2, "no line is lost: {out}");
    assert_eq!(lines[0]["currency"], json!("JPY"));
    assert_eq!(lines[1]["currency"], json!("USD"));
    assert_masked(&lines[1], &["idNumber"], "1103700012345");
}

#[test]
fn fields_that_are_not_sensitive_are_unchanged() {
    let payload = json!({
        "currency": "JPY",
        "unitSize": "100",
        "buyRate": "23.50000000",
        "sellRate": "24.10000000",
        "lines": [{ "currency": "USD", "mode": "MANUAL", "reason": "bank closed" }],
        "count": 3,
        "approved": false,
        "note": null,
    });
    assert_eq!(Redactor::redact(&payload), payload);
}
