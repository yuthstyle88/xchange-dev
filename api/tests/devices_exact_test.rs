//! Specification tests added in the rework of T13 (PR 107): the amounts on the
//! count line never pass through a floating-point number — not on the way into
//! PostgreSQL and not on the way out.
//!
//! Written before the fix; they fail on `1fea82b`. Do not edit, delete or weaken.
//!
//! They need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name
//! ends in `_test`, a role that may create databases). Without it they FAIL;
//! none skips.

mod common;
mod devices_common;
mod routes_common;

use std::str::FromStr;

use devices_common::*;
use rust_decimal::Decimal;

/// (foreign amount, rate, the product digit for digit). The first pair is the
/// one the reviewer of PR 107 measured: a 64-bit float keeps 12345669998.765432.
/// The second loses three digits as a float (…912.056).
const PAIRS: [(&str, &str, &str); 3] = [
    ("99999999.99", "123.4567", "12345669998.765433"),
    ("87654321.09", "9876.5432", "865721688912.056088"),
    ("100000000", "0.0001", "10000"),
];

async fn save(db: &Devices, n: u32, amount: &str, rate: &str) -> String {
    let submission = format!("sub-exact-{n}");
    let saved = SavedExchange {
        cash: true,
        submission_id: submission.clone(),
        document_ref: format!("XC-20261007-{n:06}"),
        currency: "USD".to_owned(),
        amount: Decimal::from_str(amount).expect("a decimal"),
        rate: Decimal::from_str(rate).expect("a decimal"),
        passport_key: None,
        slip_key: None,
    };
    db.ids.give_one(&job_id(n));
    let service = db.service.clone();
    let main = branch(MAIN);
    let inner = main.clone();
    db.branch_tx()
        .run(&main, move |tx| {
            Box::pin(async move {
                service
                    .after_save_in(tx, &inner, &saved, ACTOR, at(i64::from(n)))
                    .await
            })
        })
        .await
        .expect("the lines of a save");
    submission
}

/// What PostgreSQL holds, read back as text: every digit of the product and of
/// the amount.
#[tokio::test]
async fn the_count_line_stores_the_product_digit_for_digit() {
    let db = Devices::start().await;
    for (n, (amount, rate, product)) in PAIRS.iter().enumerate() {
        let submission = save(&db, n as u32 + 1, amount, rate).await;
        let (thb, foreign): (String, String) = sqlx::query_as(
            "SELECT detail->>'thb', detail->>'foreignAmount' FROM device_events \
             WHERE kind = 'COUNT_CONFIRMED' AND ref = $1",
        )
        .bind(&submission)
        .fetch_one(&db.pool)
        .await
        .expect("the count line");
        assert_eq!(&thb, product, "{amount} × {rate}");
        assert_eq!(&foreign, amount, "the amount of {amount} × {rate}");
    }
    db.finish().await;
}

/// What the service hands out: the two amounts as decimal text — a JSON string
/// that parses to the exact `Decimal` — never a JSON number, which this
/// workspace's `serde_json` holds as a float.
#[tokio::test]
async fn the_count_line_is_handed_out_as_exact_decimals() {
    let db = Devices::start().await;
    for (n, (amount, rate, _)) in PAIRS.iter().enumerate() {
        save(&db, n as u32 + 1, amount, rate).await;
    }
    let overview = db.service.overview(&branch(MAIN)).await.unwrap();
    let lines: Vec<&DeviceEvent> = overview
        .events
        .iter()
        .filter(|e| e.kind == "COUNT_CONFIRMED")
        .collect();
    assert_eq!(lines.len(), PAIRS.len());
    // newest first
    for (line, (amount, rate, product)) in lines.iter().zip(PAIRS.iter().rev()) {
        let thb = line.detail["thb"]
            .as_str()
            .unwrap_or_else(|| panic!("thb is decimal text, got {}", line.detail["thb"]));
        let foreign = line.detail["foreignAmount"].as_str().unwrap_or_else(|| {
            panic!(
                "foreignAmount is decimal text, got {}",
                line.detail["foreignAmount"]
            )
        });
        assert_eq!(thb, *product, "{amount} × {rate}");
        assert_eq!(foreign, *amount);
        let exact = Decimal::from_str(amount).unwrap() * Decimal::from_str(rate).unwrap();
        assert_eq!(Decimal::from_str(thb).unwrap(), exact);
        // the rest of the line is as before
        assert_eq!(line.detail["mode"], "EMPLOYEE_CONFIRMATION");
        assert_eq!(line.detail["currency"], "USD");
    }
    db.finish().await;
}
