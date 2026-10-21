//! #4137 row 5 (rate site): `validate_request_id` answers exactly as before the shared rule.
//! Green before and after #4137. Written by the BE-R6 sub-controller (claude/studio-claude-8); do not edit, delete
//! or weaken.

use pos_error::AppError;
use xc_rate::domain::validation::{BAD_REQUEST_ID, validate_request_id};

const TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";

fn refusal(raw: &str) -> String {
    match validate_request_id(raw) {
        Err(AppError::Domain(s)) => s,
        other => panic!("{raw:?}: expected a domain refusal, got {other:?}"),
    }
}

#[test]
fn the_rate_site_answers_as_before() {
    let sixty = "A1-".repeat(20);
    assert_eq!(
        validate_request_id("abcdefghij123456").ok(),
        Some("abcdefghij123456")
    );
    assert_eq!(validate_request_id(&sixty).ok(), Some(sixty.as_str()));
    assert_eq!(
        validate_request_id("  abcdefghij123456 ").ok(),
        Some("abcdefghij123456")
    );
    assert_eq!(refusal("abcdefghij12345"), BAD_REQUEST_ID);
    assert_eq!(refusal(&format!("{sixty}x")), TEXT);
    for bad in [
        "abcdefghij_123456",
        "abcdefghij.123456",
        "abcdefghijก123456",
    ] {
        assert_eq!(refusal(bad), BAD_REQUEST_ID, "{bad:?}");
    }
    assert_eq!(refusal(""), TEXT);
    assert_eq!(refusal("   "), TEXT);
}
