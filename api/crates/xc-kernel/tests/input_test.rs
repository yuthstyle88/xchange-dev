//! Written before the implementation (ticket #88, IN-K). Do not edit, delete or weaken.
//!
//! The input rules and sentences that `xc-exchange`, `xc-rate` and `xc-shift` each wrote out
//! (at `main` `6b809e2`: `customer_details.rs:48`, `:57`; `transaction_form.rs:206`, `:215`,
//! `:222`; `xc-rate/src/domain/validation.rs:33`, `:55`; `xc-shift/src/domain/shift/commands/
//! payload.rs:9`), and Bangkok's offset (`receipt.rs:55`, `document_ref.rs:11`, `:22`,
//! `custody_time.rs:16`). Every case below is what those copies answer today; the callers'
//! own tests keep holding them when they move onto this module.

use chrono::{NaiveDate, TimeZone, Utc};
use pos_error::AppError;
use xc_kernel::input::{DATE_FLOOR, InputError, calendar_date, number_error, text};
use xc_kernel::money::NumberError;
use xc_kernel::time::{BANGKOK_OFFSET_SECONDS, bangkok};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a day")
}

// ── the sentences ────────────────────────────────────────────────────────

#[test]
fn each_refusal_says_v2s_sentence() {
    for (error, sentence) in [
        (InputError::Text, "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด"),
        (InputError::DateFormat, "รูปแบบวันที่ไม่ถูกต้อง"),
        (InputError::Date, "วันที่ไม่ถูกต้อง"),
        (InputError::Number, "จำนวนเงินหรือเรทไม่ถูกต้อง"),
        (InputError::TooManyDecimals, "ทศนิยมเกินกำหนด"),
    ] {
        assert_eq!(error.sentence(), sentence, "{error:?}");
        match AppError::from(error) {
            AppError::Domain(said) => assert_eq!(said, sentence, "{error:?}"),
            other => panic!("{error:?} must be a refusal, got {other:?}"),
        }
    }
}

// ── a text ───────────────────────────────────────────────────────────────

#[test]
fn a_text_is_trimmed_and_counted_in_utf16_units() {
    assert_eq!(text("  Somchai  ", 100, true), Ok("Somchai"));
    assert_eq!(text("\t a b \n", 3, true), Ok("a b"));
    // exactly the limit, then one more
    assert_eq!(text(&"x".repeat(100), 100, true).map(str::len), Ok(100));
    assert_eq!(text(&"x".repeat(101), 100, true), Err(InputError::Text));
    // the limit counts after trimming
    assert_eq!(
        text(&format!("  {}  ", "x".repeat(10)), 10, true).map(str::len),
        Ok(10)
    );
    // Thai is one unit a letter, an emoji two
    assert!(text(&"ก".repeat(10), 10, true).is_ok());
    assert!(text(&"😀".repeat(5), 10, true).is_ok());
    assert_eq!(text(&"😀".repeat(6), 10, true), Err(InputError::Text));
}

#[test]
fn a_required_text_cannot_be_empty_and_an_optional_one_can() {
    for blank in ["", " ", "\t\n "] {
        assert_eq!(text(blank, 10, true), Err(InputError::Text), "{blank:?}");
        assert_eq!(text(blank, 10, false), Ok(""), "{blank:?}");
    }
    assert_eq!(text(&"x".repeat(11), 10, false), Err(InputError::Text));
}

// ── a calendar date ──────────────────────────────────────────────────────

#[test]
fn a_date_is_four_two_two_ascii_digits() {
    assert_eq!(calendar_date("1990-01-31"), Ok(day(1990, 1, 31)));
    for bad in [
        "1990-1-31",
        "1990/01/31",
        "31-01-1990",
        "1990-01-3a",
        "19900131",
        "１９９０-01-31",
        "1990-01-31T00",
        " 1990-01-31",
    ] {
        assert_eq!(calendar_date(bad), Err(InputError::DateFormat), "{bad:?}");
    }
}

#[test]
fn a_date_must_be_a_real_day_not_before_the_floor() {
    assert_eq!(DATE_FLOOR, day(1920, 4, 2));
    assert_eq!(calendar_date("1920-04-02"), Ok(day(1920, 4, 2)));
    for bad in [
        "1920-04-01",
        "1900-01-01",
        "1990-02-30",
        "2023-02-29",
        "1990-13-01",
        "1990-00-10",
    ] {
        assert_eq!(calendar_date(bad), Err(InputError::Date), "{bad:?}");
    }
    assert_eq!(calendar_date("2024-02-29"), Ok(day(2024, 2, 29)));
}

// ── a number ─────────────────────────────────────────────────────────────

#[test]
fn a_number_error_maps_to_its_refusal() {
    assert_eq!(number_error(NumberError::Invalid), InputError::Number);
    assert_eq!(
        number_error(NumberError::TooManyDecimals),
        InputError::TooManyDecimals
    );
}

// ── Bangkok's clock ──────────────────────────────────────────────────────

#[test]
fn bangkok_is_seven_hours_ahead_of_utc_all_year() {
    assert_eq!(BANGKOK_OFFSET_SECONDS, 7 * 3600);
    assert_eq!(bangkok().local_minus_utc(), 7 * 3600);
    // 17:00 UTC is midnight of the next day in Bangkok
    let evening = Utc
        .with_ymd_and_hms(2026, 10, 6, 17, 0, 0)
        .single()
        .unwrap();
    assert_eq!(
        evening.with_timezone(&bangkok()).date_naive(),
        day(2026, 10, 7)
    );
    let before = Utc
        .with_ymd_and_hms(2026, 10, 6, 16, 59, 59)
        .single()
        .unwrap();
    assert_eq!(
        before.with_timezone(&bangkok()).date_naive(),
        day(2026, 10, 6)
    );
    // no daylight saving: the same offset in January and in July
    for month in [1, 7] {
        let t = Utc
            .with_ymd_and_hms(2026, month, 1, 0, 0, 0)
            .single()
            .unwrap();
        assert_eq!(
            t.with_timezone(&bangkok()).offset().local_minus_utc(),
            25_200
        );
    }
}
