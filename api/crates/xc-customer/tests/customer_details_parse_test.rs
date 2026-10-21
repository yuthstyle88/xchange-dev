//! Tests for `CustomerDetails::parse` (ticket #4004, round 2 item A2).

use pos_error::{AppError, AppResult};
use xc_customer::{
    BAD_DATE, BAD_MODE, BAD_TEXT, CustomerDetails, INVALID_NATIONALITY, ReserveCustomer,
};
use xc_kernel::Mode;

fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

fn valid_customer() -> ReserveCustomer {
    ReserveCustomer {
        name: "Somchai Jaidee".to_string(),
        passport_no: "AB123456".to_string(),
        nationality: "THA".to_string(),
        birth_date: "1990-01-01".to_string(),
        passport_expiry: "2030-01-01".to_string(),
        mode: "LIVE".to_string(),
    }
}

// Row 1: nationality "TH1" -> Err whose message is INVALID_NATIONALITY
// Mutation it must catch: delete the all(is_ascii_alphabetic) check
#[test]
fn row_1_nationality_with_digit_is_invalid() {
    let mut cmd = valid_customer();
    cmd.nationality = "TH1".to_string();
    assert_eq!(refusal(CustomerDetails::parse(&cmd)), INVALID_NATIONALITY);
}

// Row 2: nationality "TH" (length < 3) -> INVALID_NATIONALITY
// Mutation it must catch: != 3 -> < 3 (or > 3)
#[test]
fn row_2_nationality_too_short_is_invalid() {
    let mut cmd = valid_customer();
    cmd.nationality = "TH".to_string();
    assert_eq!(refusal(CustomerDetails::parse(&cmd)), INVALID_NATIONALITY);
}

// Row 3: nationality "THAI" (length > 3) -> INVALID_NATIONALITY
// Mutation it must catch: != 3 -> > 3 (or < 3)
#[test]
fn row_3_nationality_too_long_is_invalid() {
    let mut cmd = valid_customer();
    cmd.nationality = "THAI".to_string();
    assert_eq!(refusal(CustomerDetails::parse(&cmd)), INVALID_NATIONALITY);
}

// Row 4: nationality "tha" -> Ok, nationality is "THA"
// Mutation it must catch: drop to_uppercase at :31
#[test]
fn row_4_nationality_lowercase_is_uppercased() {
    let mut cmd = valid_customer();
    cmd.nationality = "tha".to_string();
    let details = CustomerDetails::parse(&cmd).expect("valid details");
    assert_eq!(details.nationality, "THA");
}

// Row 5: birth date "1990-01-01x" (> 10 chars) -> BAD_TEXT (not BAD_DATE)
// Mutation it must catch: the > 10 length guard in parse_date
#[test]
fn row_5_birth_date_too_long_is_bad_text() {
    let mut cmd = valid_customer();
    cmd.birth_date = "1990-01-01x".to_string();
    assert_eq!(refusal(CustomerDetails::parse(&cmd)), BAD_TEXT);
}

// Row 6: birth date "   " (whitespace) -> Ok, birth_date = None
// Mutation it must catch: drop the trim / empty check in parse_date
#[test]
fn row_6_birth_date_whitespace_is_none() {
    let mut cmd = valid_customer();
    cmd.birth_date = "   ".to_string();
    let details = CustomerDetails::parse(&cmd).expect("valid details");
    assert_eq!(details.birth_date, None);
}

// Row 7: passport expiry "2020-02-30" (invalid calendar day) -> BAD_DATE
// Mutation it must catch: skip parsing the expiry
#[test]
fn row_7_passport_expiry_invalid_day_is_bad_date() {
    let mut cmd = valid_customer();
    cmd.passport_expiry = "2020-02-30".to_string();
    assert_eq!(refusal(CustomerDetails::parse(&cmd)), BAD_DATE);
}

// Row 8: mode "live" (lowercase) -> BAD_MODE
// Mutation it must catch: case-folding the mode
#[test]
fn row_8_mode_lowercase_is_bad_mode() {
    let mut cmd = valid_customer();
    cmd.mode = "live".to_string();
    assert_eq!(refusal(CustomerDetails::parse(&cmd)), BAD_MODE);
}

// Row 9: mode "DEMO" -> Ok, mode is Demo
// Mutation it must catch: swapped mode arms ("LIVE" => Demo, "DEMO" => Live)
#[test]
fn row_9_mode_demo_parsed_as_demo() {
    let mut cmd = valid_customer();
    cmd.mode = "DEMO".to_string();
    let details = CustomerDetails::parse(&cmd).expect("valid details");
    assert_eq!(details.mode, Mode::Demo);
}

// Row 10: name of 100 chars (Ok) / 101 chars (BAD_TEXT)
// Mutation it must catch: max 100 -> 101 (:29)
#[test]
fn row_10_name_length_boundary_100_and_101() {
    let mut cmd = valid_customer();
    let name_100 = "a".repeat(100);
    cmd.name = name_100.clone();
    let details = CustomerDetails::parse(&cmd).expect("100 chars is valid");
    assert_eq!(details.name, name_100);

    cmd.name = "a".repeat(101);
    assert_eq!(refusal(CustomerDetails::parse(&cmd)), BAD_TEXT);
}

// Row 11: passport of 30 chars (Ok) / 31 chars (BAD_TEXT)
// Mutation it must catch: max 30 -> 31 (:30)
#[test]
fn row_11_passport_length_boundary_30_and_31() {
    let mut cmd = valid_customer();
    let passport_30 = "A".repeat(30);
    cmd.passport_no = passport_30.clone();
    let details = CustomerDetails::parse(&cmd).expect("30 chars is valid");
    assert_eq!(details.passport_no, passport_30);

    cmd.passport_no = "A".repeat(31);
    assert_eq!(refusal(CustomerDetails::parse(&cmd)), BAD_TEXT);
}

// Row 12: mode "X" and nationality "TH1" -> BAD_MODE
// Mutation it must catch: mode checked after nationality
#[test]
fn row_12_mode_checked_before_nationality() {
    let mut cmd = valid_customer();
    cmd.mode = "X".to_string();
    cmd.nationality = "TH1".to_string();
    assert_eq!(refusal(CustomerDetails::parse(&cmd)), BAD_MODE);
}
