//! Written before the implementation (ticket T15-a, #407). Do not edit, delete or weaken.
//!
//! The booking domain's values, in design `docs/design/xchange-v3.md` §5.6.3 and §5.6.5:
//! `BookingStatus`, `BookingAction`, `BookingRules`, the input functions of `booking_input`
//! and the sentences of `BookingError`.
//!
//! The answers are v2's (10-05 bundle `.dev/v2/server.mjs`; the design cites the 10-06 bundle,
//! eight lines later): `text` `:7167-7171`, `phone` `:7172-7176`, the member secret `:7233-7234`,
//! `settings` `:7225-7226`. Where v3 differs on purpose the design's §5.6.7 says so (a text field
//! given as a number, `minutes` given as text: v2 coerced them, v3 refuses).
//!
//! No database is used here.

use serde_json::{Value, json};

use xc_exchange::domain::booking::value_objects::booking_action::BookingAction;
use xc_exchange::domain::booking::value_objects::booking_error::BookingError;
use xc_exchange::domain::booking::value_objects::booking_input::{
    contact_phone, member_secret, required_text, token_hash,
};
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_exchange::domain::booking::value_objects::booking_status::BookingStatus;

const FILL_IN: &str = "กรอกข้อมูลให้ครบและไม่เกินกำหนด";
const BAD_PHONE: &str = "เบอร์ติดต่อไม่ถูกต้อง";
const BAD_MEMBER_CODE: &str = "รหัสสมาชิกไม่ถูกต้อง";
// U+2013 EN DASH between the two numbers, as v2 writes it.
const BAD_RULES: &str = "กำหนดเวลา 5\u{2013}1440 นาที";

/// The sentence and the status of a refusal; panics when the value was accepted.
fn refused<T>(got: Result<T, BookingError>) -> (String, u16) {
    match got {
        Ok(_) => panic!("must be refused"),
        Err(e) => (e.to_string(), e.status()),
    }
}

fn accepted<T>(got: Result<T, BookingError>) -> T {
    match got {
        Ok(v) => v,
        Err(e) => panic!("must be accepted, was refused: {e}"),
    }
}

// ── statuses ─────────────────────────────────────────────────────────────────

#[test]
fn the_four_stored_statuses_read_and_print_as_v2_writes_them() {
    for (status, text) in [
        (BookingStatus::Pending, "PENDING"),
        (BookingStatus::Confirmed, "CONFIRMED"),
        (BookingStatus::Used, "USED"),
        (BookingStatus::Cancelled, "CANCELLED"),
    ] {
        assert_eq!(status.as_str(), text);
        assert_eq!(BookingStatus::parse(text), Some(status), "{text}");
    }
}

#[test]
fn expired_is_shown_never_stored_and_no_other_spelling_parses() {
    // EXPIRED is computed for display (`publicBooking`), never written (§5.6.3 *States*)
    for text in ["EXPIRED", "pending", "Confirmed", "USED ", "", "CANCELED"] {
        assert_eq!(BookingStatus::parse(text), None, "{text:?}");
    }
}

// ── actions ──────────────────────────────────────────────────────────────────

#[test]
fn the_nine_action_names_are_v2s_exact_names() {
    for (text, action, owner_only) in [
        ("list", BookingAction::List, false),
        ("get", BookingAction::Get, false),
        ("create", BookingAction::Create, false),
        ("confirm", BookingAction::Confirm, false),
        ("cancel", BookingAction::Cancel, false),
        ("member", BookingAction::Member, true),
        ("settings", BookingAction::Settings, true),
        ("registrations", BookingAction::Registrations, true),
        (
            "approveRegistration",
            BookingAction::ApproveRegistration,
            true,
        ),
    ] {
        let parsed = BookingAction::parse(text);
        assert!(parsed == Some(action), "{text} parses to its action");
        assert_eq!(action.owner_only(), owner_only, "{text}");
    }
}

#[test]
fn any_other_name_is_no_action() {
    for text in [
        "",
        "List",
        "CREATE",
        "cancel ",
        "approveregistration",
        "approve_registration",
        "delete",
        "use",
        "redeem",
    ] {
        assert!(BookingAction::parse(text).is_none(), "{text:?}");
    }
}

// ── rules ────────────────────────────────────────────────────────────────────

#[test]
fn the_default_rules_are_enabled_for_thirty_minutes() {
    // v2 answers `{enabled: 1, minutes: 30}` before a branch's first save (§5.6.6 item 2)
    let rules = BookingRules::DEFAULT;
    assert!(rules.enabled);
    assert_eq!(BookingRules::DEFAULT.minutes, 30);
}

#[test]
fn rules_parse() {
    for (enabled, minutes) in [
        (json!(true), json!(5)),
        (json!(false), json!(1440)),
        (json!(true), json!(30)),
        (json!(false), json!(45)),
    ] {
        let rules = accepted(BookingRules::parse(&enabled, &minutes));
        assert_eq!(rules.enabled, enabled == json!(true), "{enabled} {minutes}");
        assert_eq!(
            Some(u64::from(rules.minutes)),
            minutes.as_u64(),
            "{enabled} {minutes}"
        );
    }

    // §5.6.8: `minutes: 4`, `1441`, `30.5`, `"30"`, or `enabled: 1`
    let refusals: [(Value, Value); 12] = [
        (json!(true), json!(4)),
        (json!(true), json!(1441)),
        (json!(true), json!(0)),
        (json!(true), json!(-30)),
        (json!(true), json!(30.5)),
        (json!(true), json!("30")),
        (json!(true), Value::Null),
        (json!(true), json!(65_536 + 30)),
        (json!(1), json!(30)),
        (json!("true"), json!(30)),
        (Value::Null, json!(30)),
        (json!([true]), json!(30)),
    ];
    for (enabled, minutes) in refusals {
        assert_eq!(
            refused(BookingRules::parse(&enabled, &minutes)),
            (BAD_RULES.to_owned(), 400),
            "enabled {enabled}, minutes {minutes}"
        );
    }
}

// ── text ─────────────────────────────────────────────────────────────────────

#[test]
fn required_text_is_a_trimmed_json_string_within_the_limit() {
    assert_eq!(accepted(required_text(&json!("  BK-1 "), 60)), "BK-1");
    assert_eq!(
        accepted(required_text(&json!("a".repeat(60)), 60)),
        "a".repeat(60)
    );
    // Thai letters are one UTF-16 unit each
    assert_eq!(
        accepted(required_text(&json!("สมชาย ใจดี"), 100)),
        "สมชาย ใจดี"
    );
    // 30 emoji are 60 UTF-16 units: v2 counts `.length`
    assert_eq!(
        accepted(required_text(&json!("😀".repeat(30)), 60)),
        "😀".repeat(30)
    );

    for v in [
        json!(""),
        json!("   "),
        json!("a".repeat(61)),
        json!("😀".repeat(31)),
        json!(format!(" {} ", "a".repeat(61))),
        // §5.6.7: a text field given as a number is refused (v2 coerced it with `String()`)
        json!(5),
        json!(true),
        Value::Null,
        json!(["BK-1"]),
        json!({ "id": "BK-1" }),
    ] {
        assert_eq!(
            refused(required_text(&v, 60)),
            (FILL_IN.to_owned(), 400),
            "{v}"
        );
    }
}

#[test]
fn the_limit_is_the_callers() {
    // `get`'s id is ≤ 60 (§5.6.3), `create`'s name ≤ 100, the member secret ≤ 80
    let ninety = json!("ก".repeat(90));
    assert_eq!(refused(required_text(&ninety, 60)).0, FILL_IN);
    assert_eq!(accepted(required_text(&ninety, 100)), "ก".repeat(90));
}

// ── phone ────────────────────────────────────────────────────────────────────

#[test]
fn a_contact_phone_is_seven_digits_or_more_of_plus_digits_spaces_brackets_and_dashes() {
    for (raw, kept) in [
        ("081-234-5678", "081-234-5678"),
        (" +66 (81) 234 5678 ", "+66 (81) 234 5678"),
        ("1234567", "1234567"),
        ("(02) 123-4567", "(02) 123-4567"),
        (
            "+12----------------------34567",
            "+12----------------------34567",
        ),
    ] {
        assert_eq!(accepted(contact_phone(&json!(raw))), kept, "{raw:?}");
    }
}

#[test]
fn a_phone_with_other_characters_or_fewer_than_seven_digits_is_refused() {
    for raw in [
        "123456",
        "12-34-56",
        "081x2345678",
        "081.234.5678",
        "๐๘๑๒๓๔๕๖๗๘",
        "+66 81 234 5678 ext",
        "(((((())))))",
    ] {
        assert_eq!(
            refused(contact_phone(&json!(raw))),
            (BAD_PHONE.to_owned(), 400),
            "{raw:?}"
        );
    }
}

#[test]
fn a_phone_that_is_not_a_json_string_is_refused() {
    // §5.6.7: text fields given as numbers are refused, a phone included
    for v in [json!(812_345_678), Value::Null, json!(["0812345678"])] {
        assert!(contact_phone(&v).is_err(), "{v}");
    }
}

// ── member secret ────────────────────────────────────────────────────────────

#[test]
fn a_member_secret_is_thirty_six_of_lower_hex_and_dashes() {
    let code = "6f1c2c1e-5a3b-4c7d-9e8f-0a1b2c3d4e5f";
    assert_eq!(accepted(member_secret(&json!(code))), code);
    assert_eq!(accepted(member_secret(&json!(format!("  {code}  ")))), code);

    for raw in [
        "6F1C2C1E-5A3B-4C7D-9E8F-0A1B2C3D4E5F",
        "6f1c2c1e-5a3b-4c7d-9e8f-0a1b2c3d4e5",
        "6f1c2c1e-5a3b-4c7d-9e8f-0a1b2c3d4e5f0",
        "6f1c2c1e 5a3b 4c7d 9e8f 0a1b2c3d4e5f",
        "6f1c2c1e-5a3b-4c7d-9e8f-0a1b2c3d4e5g",
    ] {
        assert_eq!(
            refused(member_secret(&json!(raw))),
            (BAD_MEMBER_CODE.to_owned(), 400),
            "{raw:?}"
        );
    }
}

#[test]
fn a_member_secret_is_text_of_at_most_eighty_first() {
    // member rule 3 (text ≤ 80) comes before rule 4 (the format), as v2 `:7233-7234`
    for v in [json!(""), json!("a".repeat(81)), json!(12345), Value::Null] {
        assert_eq!(refused(member_secret(&v)), (FILL_IN.to_owned(), 400), "{v}");
    }
    assert_eq!(
        refused(member_secret(&json!("a".repeat(80)))).0,
        BAD_MEMBER_CODE
    );
}

#[test]
fn the_token_hash_is_the_sha256_hex_of_the_code_alone() {
    // §5.6.2 [proposed]: no bridge secret in v3, the hash of the code itself
    assert_eq!(
        token_hash("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        token_hash("6f1c2c1e-5a3b-4c7d-9e8f-0a1b2c3d4e5f"),
        "7c15e2bdbcfc71016f5042c55625e9ece4cf7db1a4124bc5670ef388f8fc3d2a"
    );
}
