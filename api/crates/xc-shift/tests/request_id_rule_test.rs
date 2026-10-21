//! #4137 row 5 (shift site): `OpenShift::from_payload` reads `requestId` exactly as before the shared rule,
//! including a JSON number (BE-R6 ruling on #4137: the numeric read stays, as a step before the shared check).
//! Green before and after #4137. Written by the BE-R6 sub-controller (claude/studio-claude-8); do not edit, delete
//! or weaken.

use serde_json::{Value, json};
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;

/// The request-id refusal of `payload`, or `None` when the request id was accepted (a later field may still fail).
fn request_id_refusal(request_id: Value) -> Option<ShiftError> {
    match OpenShift::from_payload(&json!({ "requestId": request_id, "opening": {} })) {
        Ok(_) => None,
        Err(e @ (ShiftError::TextInvalid | ShiftError::RequestIdInvalid)) => Some(e),
        Err(_) => None,
    }
}

#[test]
fn the_shift_site_answers_as_before() {
    let sixty = "A1-".repeat(20);
    for ok in [
        json!("abcdefghij123456"),
        json!(sixty),
        json!("  abcdefghij123456 "),
        json!(1234567890123456u64),
    ] {
        assert_eq!(request_id_refusal(ok.clone()), None, "{ok}");
    }
    for (bad, want) in [
        (json!("abcdefghij12345"), ShiftError::RequestIdInvalid),
        (json!(format!("{sixty}x")), ShiftError::TextInvalid),
        (json!("abcdefghij_123456"), ShiftError::RequestIdInvalid),
        (json!("abcdefghij.123456"), ShiftError::RequestIdInvalid),
        (json!("abcdefghijก123456"), ShiftError::RequestIdInvalid),
        (json!(123), ShiftError::RequestIdInvalid),
        (json!(""), ShiftError::TextInvalid),
        (Value::Null, ShiftError::TextInvalid),
        (json!(true), ShiftError::TextInvalid),
    ] {
        assert_eq!(request_id_refusal(bad.clone()), Some(want), "{bad}");
    }
}
