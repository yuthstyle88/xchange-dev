//! #4137 rows 1–4: one client request-id rule, `xc_kernel::input::request_id`, next to `text`.
//!
//! Written before the implementation by the BE-R6 sub-controller (claude/studio-claude-8). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! [required] `pub fn request_id(raw: &str) -> Result<&str, InputError>`, re-exported from `xc_kernel::input`:
//! `text(raw, 60, true)` first (trimmed; empty or over 60 UTF-16 units ⇒ `InputError::Text`), then at least 16
//! characters, all `[A-Za-z0-9-]` ⇒ else `InputError::RequestId` (sentence `รหัสคำขอไม่ถูกต้อง`).

use xc_kernel::input::{InputError, request_id};

const SIXTEEN: &str = "abcdefghij123456";

/// Row 1: lengths 15 / 16 / 60 / 61.
#[test]
fn a_request_id_is_sixteen_to_sixty_characters() {
    assert_eq!(
        request_id("abcdefghij12345"),
        Err(InputError::RequestId),
        "15"
    );
    assert_eq!(request_id(SIXTEEN), Ok(SIXTEEN), "16");
    let sixty = "A1-".repeat(20);
    assert_eq!(request_id(&sixty), Ok(sixty.as_str()), "60");
    let sixty_one = format!("{sixty}x");
    assert_eq!(
        request_id(&sixty_one),
        Err(InputError::Text),
        "61 is the text error, as today"
    );
}

/// Row 2: padded with spaces, trimmed, then checked.
#[test]
fn a_padded_request_id_is_trimmed_then_checked() {
    assert_eq!(request_id("  abcdefghij123456\t "), Ok(SIXTEEN));
    assert_eq!(
        request_id("   abcdefghij12345   "),
        Err(InputError::RequestId),
        "15 once trimmed"
    );
}

/// Row 3: `_`, `.`, a space inside, or non-ASCII.
#[test]
fn only_ascii_letters_digits_and_hyphens_are_allowed() {
    for bad in [
        "abcdefghij_123456",
        "abcdefghij.123456",
        "abcdefghij 123456",
        "abcdefghijก123456",
        "abcdefghij１23456",
    ] {
        assert_eq!(request_id(bad), Err(InputError::RequestId), "{bad:?}");
    }
    assert_eq!(request_id("ABC-def-0123456789"), Ok("ABC-def-0123456789"));
}

/// Row 4: empty is the text error, not the shape error.
#[test]
fn an_empty_request_id_is_the_text_error() {
    assert_eq!(request_id(""), Err(InputError::Text));
    assert_eq!(request_id("    "), Err(InputError::Text));
}

/// The new refusal's sentence is the one the three sites' shape refusals already say where they say one.
#[test]
fn the_shape_refusal_has_the_request_id_sentence() {
    assert_eq!(InputError::RequestId.sentence(), "รหัสคำขอไม่ถูกต้อง");
}
