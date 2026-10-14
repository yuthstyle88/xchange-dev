//! Written before the implementation by the test writer gemini/studio-9 (ticket author claude/e92db6). Do not edit, delete or weaken.
//!
//! NT-1 (#1629): `NotificationParams` and `DisplayValue` tests covering every row of design §4.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use xc_kernel::notification_params::{
    DisplayValue, DisplayValueError, NotificationParams, queue_number_text,
};
use xc_kernel::notifications::{NotificationRequest, NotificationRequestError};

#[test]
fn new_with_four_digits_display_number_is_refused_unchanged() {
    let mut params = BTreeMap::new();
    params.insert("displayNumber".to_string(), "W1000".to_string());
    let err = NotificationRequest::new("user-1", "ev-1", "queue_called", None, params).unwrap_err();
    assert_eq!(
        err,
        NotificationRequestError::PersonalData("displayNumber".to_string())
    );
}

#[test]
fn with_params_display_queue_number_1000_is_accepted() {
    let dv = DisplayValue::queue_number('W', 1000).expect("W1000 is a valid display value");
    let params = NotificationParams::new().display("displayNumber", dv);
    let req = NotificationRequest::with_params("user-1", "ev-1", "queue_called", None, params)
        .expect("server-produced display number W1000 is accepted");
    assert_eq!(
        req.params().get("displayNumber").map(String::as_str),
        Some("W1000")
    );
}

#[test]
fn queue_number_formats_padding_to_three_digits() {
    assert_eq!(DisplayValue::queue_number('W', 7).unwrap().as_str(), "W007");
    assert_eq!(queue_number_text('W', 7), "W007");
}

#[test]
fn queue_number_text_formats_without_clamping_including_negative() {
    assert_eq!(queue_number_text('W', 1000), "W1000");
    assert_eq!(queue_number_text('B', 12), "B012");
    assert_eq!(queue_number_text('W', -1), "W-01");
}

#[test]
fn queue_number_refuses_invalid_letters() {
    assert_eq!(
        DisplayValue::queue_number('w', 1),
        Err(DisplayValueError::QueueLetter('w'))
    );
    assert_eq!(
        DisplayValue::queue_number('1', 1),
        Err(DisplayValueError::QueueLetter('1'))
    );
    assert_eq!(
        DisplayValue::queue_number('$', 1),
        Err(DisplayValueError::QueueLetter('$'))
    );
}

#[test]
fn queue_number_refuses_out_of_range_sequences() {
    assert_eq!(
        DisplayValue::queue_number('W', 0),
        Err(DisplayValueError::QueueSequence(0))
    );
    assert_eq!(
        DisplayValue::queue_number('W', -1),
        Err(DisplayValueError::QueueSequence(-1))
    );
    assert_eq!(
        DisplayValue::queue_number('W', 100_000),
        Err(DisplayValueError::QueueSequence(100_000))
    );
}

#[test]
fn queue_number_accepts_max_sequence_99_999() {
    assert_eq!(
        DisplayValue::queue_number('W', 99_999).unwrap().as_str(),
        "W99999"
    );
    assert_eq!(queue_number_text('W', 99_999), "W99999");
}

#[test]
fn with_params_text_refuses_phone_and_code() {
    let params1 = NotificationParams::new().text("note", "+66812345678");
    assert_eq!(
        NotificationRequest::with_params("user-1", "ev-1", "queue_called", None, params1)
            .unwrap_err(),
        NotificationRequestError::PersonalData("note".to_string())
    );

    let params2 = NotificationParams::new().text("note", "รหัส 123456");
    assert_eq!(
        NotificationRequest::with_params("user-1", "ev-1", "queue_called", None, params2)
            .unwrap_err(),
        NotificationRequestError::PersonalData("note".to_string())
    );
}

#[test]
fn with_params_text_accepts_safe_text() {
    let params = NotificationParams::new().text("branchName", "Siam");
    let req = NotificationRequest::with_params("user-1", "ev-1", "queue_called", None, params)
        .expect("safe text is accepted");
    assert_eq!(
        req.params().get("branchName").map(String::as_str),
        Some("Siam")
    );
}

#[test]
fn display_value_count_and_date_strings() {
    let count_val = DisplayValue::count(12345);
    assert_eq!(count_val.as_str(), "12345");

    let date_val = DisplayValue::date(NaiveDate::from_ymd_opt(2026, 10, 14).unwrap());
    assert_eq!(date_val.as_str(), "2026-10-14");

    let params = NotificationParams::new()
        .display("count", count_val)
        .display("date", date_val);
    let req = NotificationRequest::with_params("user-1", "ev-1", "t", None, params)
        .expect("count and date display values are accepted");
    assert_eq!(req.params().get("count").map(String::as_str), Some("12345"));
    assert_eq!(
        req.params().get("date").map(String::as_str),
        Some("2026-10-14")
    );
}

#[test]
fn display_value_amount_and_rate_strings() {
    let amount = Decimal::from_str_exact("12500.50").unwrap();
    let rate = Decimal::from_str_exact("34.1250").unwrap();

    let amount_val = DisplayValue::amount(amount);
    assert_eq!(amount_val.as_str(), "12500.50");

    let rate_val = DisplayValue::rate(rate);
    assert_eq!(rate_val.as_str(), "34.1250");

    let params = NotificationParams::new()
        .display("amount", amount_val)
        .display("rate", rate_val);
    let req = NotificationRequest::with_params("user-1", "ev-1", "t", None, params)
        .expect("amount and rate display values are accepted");
    assert_eq!(
        req.params().get("amount").map(String::as_str),
        Some("12500.50")
    );
    assert_eq!(
        req.params().get("rate").map(String::as_str),
        Some("34.1250")
    );
}

#[test]
fn with_params_refuses_empty_required_fields() {
    assert_eq!(
        NotificationRequest::with_params("", "e:1", "t", None, NotificationParams::new())
            .unwrap_err(),
        NotificationRequestError::Empty("recipient_user_id")
    );
    assert_eq!(
        NotificationRequest::with_params("user-1", "", "t", None, NotificationParams::new())
            .unwrap_err(),
        NotificationRequestError::Empty("event_id")
    );
    assert_eq!(
        NotificationRequest::with_params("user-1", "e:1", "", None, NotificationParams::new())
            .unwrap_err(),
        NotificationRequestError::Empty("template_key")
    );
}
