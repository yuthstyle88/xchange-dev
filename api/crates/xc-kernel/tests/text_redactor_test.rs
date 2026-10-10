//! APP-2 (#558): `redact_text`, design `customer-app.md` P12 and §9.1 row APP-2. Written before the
//! implementation by the ticket author (claude/5edf7bb5). Do not edit, delete or weaken; add tests in
//! a file of your own.
//!
//! The mask itself is not pinned (any replacement will do). What is pinned: the secret is gone, no run
//! of 4 or more digits survives, the words around it survive, and text with no personal data comes back
//! unchanged.

use xc_kernel::pii::redact_text;

fn longest_digit_run(text: &str) -> usize {
    let mut best = 0;
    let mut run = 0;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

fn assert_masked(input: &str, secret: &str) {
    let out = redact_text(input);
    assert!(
        !out.contains(secret),
        "redact_text({input:?}) = {out:?} still holds {secret:?}"
    );
    assert!(
        longest_digit_run(&out) < 4,
        "redact_text({input:?}) = {out:?} still holds a run of 4 or more digits"
    );
    assert!(
        out.starts_with("before ") && out.ends_with(" after"),
        "redact_text({input:?}) = {out:?} must keep the words around the secret"
    );
}

#[test]
fn an_international_phone_number_is_masked() {
    assert_masked("before +66812345678 after", "66812345678");
}

#[test]
fn a_local_phone_number_is_masked() {
    assert_masked("before 0812345678 after", "0812345678");
}

#[test]
fn an_email_address_is_masked() {
    let out = redact_text("before a@b.co after");
    assert!(!out.contains("a@b.co") && !out.contains('@'), "{out:?}");
    assert!(
        out.starts_with("before ") && out.ends_with(" after"),
        "{out:?}"
    );
}

#[test]
fn a_passport_like_token_is_masked() {
    let out = redact_text("before AA1234567 after");
    assert!(
        !out.contains("AA1234567") && !out.contains("1234567"),
        "{out:?}"
    );
    assert!(
        out.starts_with("before ") && out.ends_with(" after"),
        "{out:?}"
    );
}

#[test]
fn a_thai_national_id_is_masked() {
    assert_masked("before 1101700230708 after", "1101700230708");
}

#[test]
fn a_card_number_is_masked() {
    assert_masked("before 4111111111111111 after", "4111111111111111");
}

#[test]
fn an_otp_like_code_is_masked() {
    assert_masked("before 246810 after", "246810");
}

#[test]
fn every_kind_in_one_message_is_masked() {
    let input = "otp 246810, call 0812345678 or +66812345678, mail a@b.co, passport AA1234567, \
                 id 1101700230708, card 4111111111111111";
    let out = redact_text(input);
    for secret in [
        "246810",
        "0812345678",
        "66812345678",
        "a@b.co",
        "AA1234567",
        "1101700230708",
        "4111111111111111",
    ] {
        assert!(!out.contains(secret), "{out:?} still holds {secret:?}");
    }
    assert!(longest_digit_run(&out) < 4, "{out:?}");
    assert!(out.starts_with("otp "), "{out:?}");
}

#[test]
fn amounts_and_currencies_are_left_intact() {
    for text in [
        "USD 500",
        "100 JPY",
        "I want USD 500 at Siam",
        "แลก USD 500 ที่สาขาสยาม",
        "我要换 100 JPY",
        "",
    ] {
        assert_eq!(redact_text(text), text, "{text:?} holds no personal data");
    }
}
