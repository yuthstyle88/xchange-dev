//! Tests for non-string values under deny-listed keys in `Redactor` (#596).

use serde_json::json;
use xc_approval::domain::redactor::Redactor;

#[test]
fn a_number_phone_field_is_masked_keeping_last_four_digits() {
    let out = Redactor::redact(&json!({ "phone": 66812345678u64 }));
    assert_eq!(out, json!({ "phone": "***5678" }));
}

#[test]
fn a_number_id_number_field_is_masked_keeping_last_four_digits() {
    let out = Redactor::redact(&json!({ "idNumber": 1103700012345u64 }));
    assert_eq!(out, json!({ "idNumber": "***2345" }));
}

#[test]
fn a_bool_under_masked_key_is_masked_keeping_last_four() {
    let out = Redactor::redact(&json!({ "phone": true }));
    assert_eq!(out, json!({ "phone": "***true" }));
}

#[test]
fn an_object_or_array_under_masked_key_is_fully_masked_and_null_stays() {
    let out = Redactor::redact(&json!({
        "phone": { "country": 66, "number": 812345678 },
        "idNumber": [1, 2, 3],
        "passport": null,
    }));
    assert_eq!(
        out,
        json!({
            "phone": "***",
            "idNumber": "***",
            "passport": null,
        })
    );
}
