//! Up-front tests for the `redact_text` follow-up of APP-2 (design `customer-app.md` P12; PR 1061
//! review issuecomment-6096258058): forms of personal data the first tests did not pin. Written by the
//! ticket author before the fix. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The rules pinned here:
//! - A **digit group** is digit runs joined by a single `-` or a single space. A group of 6 or more
//!   digits in total is masked whole, so no part of a separated phone number, id or spaced OTP
//!   survives. Commas and decimal points do not join (`1,500`, `12.50` are amounts).
//! - **Thai (`๐`–`๙`) and full-width (`０`–`９`) digits** are digits, for every rule.
//! - **E-mail:** the `@` may have one space on each side, and the local part may hold any letter
//!   (Thai included). The domain still needs a dot, so `see you @ Siam` is not an address.
//!
//! "Digit" below means any `char::is_numeric` digit, so a Thai or full-width run counts.

use xc_kernel::pii::redact_text;

fn longest_digit_run_ignoring_separators(text: &str) -> usize {
    // Digits joined by a single '-' or ' ' count as one run, as the rule does.
    let chars: Vec<char> = text.chars().collect();
    let mut best = 0;
    let mut run = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_numeric() {
            run += 1;
            best = best.max(run);
        } else if (c == '-' || c == ' ')
            && run > 0
            && chars.get(i + 1).is_some_and(|n| n.is_numeric())
        {
            // joiner: the run continues
        } else {
            run = 0;
        }
        i += 1;
    }
    best
}

fn digits_of(s: &str) -> String {
    s.chars().filter(|c| c.is_numeric()).collect()
}

fn assert_gone(input: &str, secret: &str) {
    let out = redact_text(input);
    assert!(
        !out.contains(secret),
        "redact_text({input:?}) = {out:?} still holds {secret:?}"
    );
    // No tail of the secret survives: none of its digit groups of 3+ is left in the output.
    for part in secret.split(['-', ' ']) {
        if digits_of(part).chars().count() >= 3 {
            assert!(
                !out.contains(part),
                "redact_text({input:?}) = {out:?} still holds the part {part:?} of {secret:?}"
            );
        }
    }
    assert!(
        longest_digit_run_ignoring_separators(&out) < 4,
        "redact_text({input:?}) = {out:?} still holds a digit group of 4 or more"
    );
    // The words around the secret hold no digit, so no digit at all may survive: a group is
    // masked whole, never down to a tail like `1-***-***-70-8`.
    assert!(
        !out.chars().any(char::is_numeric),
        "redact_text({input:?}) = {out:?} still holds a digit of {secret:?}"
    );
    assert!(
        out.starts_with("before ") && out.ends_with(" after"),
        "redact_text({input:?}) = {out:?} must keep the words around the secret"
    );
}

#[test]
fn a_dashed_phone_number_is_masked_whole() {
    assert_gone("before 081-234-5678 after", "081-234-5678");
}

#[test]
fn a_spaced_phone_number_is_masked_whole() {
    assert_gone("before 081 234 5678 after", "081 234 5678");
}

#[test]
fn a_spaced_international_phone_number_is_masked_whole() {
    assert_gone("before +66 81 234 5678 after", "+66 81 234 5678");
}

#[test]
fn a_dashed_thai_national_id_is_masked_whole() {
    assert_gone("before 1-1017-00230-70-8 after", "1-1017-00230-70-8");
}

#[test]
fn an_otp_typed_with_a_space_is_masked() {
    assert_gone("before 123 456 after", "123 456");
}

#[test]
fn a_phone_number_in_thai_digits_is_masked() {
    assert_gone("before ๐๘๑๒๓๔๕๖๗๘ after", "๐๘๑๒๓๔๕๖๗๘");
}

#[test]
fn a_dashed_phone_number_in_thai_digits_is_masked_whole() {
    assert_gone("before ๐๘๑-๒๓๔-๕๖๗๘ after", "๐๘๑-๒๓๔-๕๖๗๘");
}

#[test]
fn a_phone_number_in_full_width_digits_is_masked() {
    assert_gone("before ０８１２３４５６７８ after", "０８１２３４５６７８");
}

#[test]
fn an_otp_in_thai_digits_is_masked() {
    assert_gone("before ๒๔๖๘๑๐ after", "๒๔๖๘๑๐");
}

#[test]
fn an_email_with_spaces_around_the_at_sign_is_masked() {
    let out = redact_text("before a @ b.co after");
    assert!(
        !out.contains("b.co") && !out.contains('@'),
        "{out:?} still holds the address"
    );
    assert!(
        out.starts_with("before ") && out.ends_with(" after"),
        "{out:?}"
    );
}

#[test]
fn an_email_with_a_thai_local_part_is_masked() {
    let out = redact_text("before ทดสอบ@b.co after");
    assert!(
        !out.contains("ทดสอบ") && !out.contains("b.co") && !out.contains('@'),
        "{out:?} still holds the address"
    );
    assert!(
        out.starts_with("before ") && out.ends_with(" after"),
        "{out:?}"
    );
}

#[test]
fn a_thai_message_around_a_dashed_phone_keeps_its_words() {
    let out = redact_text("โทร 081-234-5678 ได้เลย");
    assert!(!out.chars().any(char::is_numeric), "{out:?}");
    assert!(
        out.starts_with("โทร ") && out.ends_with(" ได้เลย"),
        "{out:?}"
    );
}

#[test]
fn amounts_dates_and_short_numbers_are_left_intact() {
    for text in [
        "USD 500",
        "แลก USD 500 ที่สาขาสยาม",
        "แลก ๕๐๐ ดอลลาร์",
        "1,500 THB",
        "1,500,000 บาท",
        "rate 32.50",
        "12.50 - 13.75",
        "rate 12.50 13.75",
        "100 JPY and 200 EUR",
        "queue A12",
        "see you @ Siam",
        "@ 5pm",
        "",
    ] {
        assert_eq!(redact_text(text), text, "{text:?} holds no personal data");
    }
}
