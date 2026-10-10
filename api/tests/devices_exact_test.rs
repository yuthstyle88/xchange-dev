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

/// (foreign amount, rate, the baht digit for digit). The first pair is the
/// one the reviewer of PR 107 measured: a 64-bit float keeps 12345669998.765432.
/// The second loses three digits as a float (…912.056).
///
/// Amended for #759 (RE-4e, rates.md §4.9 R2; ticket author's ruling): the count
/// line's baht is `ExchangeArithmetic::thb`, HALF_UP to 2 places, not the exact
/// product (12345669998.765433 → .77, 865721688912.056088 → .06; 10000 stays 10000).
/// Still never through a float; the text is normalized as before (`amounts_json`).
const PAIRS: [(&str, &str, &str); 3] = [
    ("99999999.99", "123.4567", "12345669998.77"),
    ("87654321.09", "9876.5432", "865721688912.06"),
    ("100000000", "0.0001", "10000"),
];

/// The `transactions` row the save writes before its device step (amended for #759, RE-4e:
/// rates.md §4.9 R9 and the ticket author's ruling, the device service reads the stored row's
/// `unit_size` by document number, and a missing row is an error). The rate is per 1, as the
/// saved exchange carries it, so `unit_size` is 1; the principal is the baht HALF_UP to 2
/// places. A customer and an open shift of the branch come with it. A row already there (the
/// same document saved again) is kept.
async fn the_saved_row(
    pool: &sqlx::PgPool,
    branch_id: &str,
    document_ref: &str,
    submission_id: &str,
    currency: &str,
    amount: &str,
    rate: &str,
) {
    sqlx::query(
        // CI-4 (#651): a customer belongs to the organisation
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'dev-customer-' || $1, organization_id, 'dev-customer-' || $1, 'Somchai', 'LIVE', now() \
         FROM branches WHERE id = $1 ON CONFLICT DO NOTHING",
    )
    .bind(branch_id)
    .execute(pool)
    .await
    .expect("the saved row's customer");
    // The shift's teller (`shifts.teller_id` is NOT NULL since #753).
    sqlx::query(
        "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
         SELECT id, 'dev-teller', 'INVITED', 'test', 'dev-teller' FROM organizations \
         WHERE code = 'XC' ON CONFLICT (legacy_employee_id) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("the saved row's teller");
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ('dev-shift-' || $1, $1, 'OPEN', now(), 'owner', 'dev-open-' || $1, \
           (SELECT id FROM users WHERE legacy_employee_id = 'dev-teller')) \
         ON CONFLICT DO NOTHING",
    )
    .bind(branch_id)
    .execute(pool)
    .await
    .expect("the saved row's shift");
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, \
            payload_hash, customer_id, customer_name, nationality, type, code, amount, rate, \
            unit_size, principal, fee, bank_fee, received, paid, method, payment_ref, account, \
            staff_id, booking_no, notes, value_date, paid_at, recorded_at, recorded_by) \
         VALUES ($1, $2, 'LIVE', 'dev-shift-' || $2, $3, 'hash', 'dev-customer-' || $2, \
            'Somchai', 'THA', 'BUY', $4, $5::numeric, $6::numeric, 1, \
            round($5::numeric * $6::numeric, 2), 0, 0, 0, round($5::numeric * $6::numeric, 2), \
            'Cash', '', 'Cash drawer', 'owner', 'WALK-IN', '', '2026-10-06', now(), now(), \
            'owner') \
         ON CONFLICT (document_ref) DO NOTHING",
    )
    .bind(document_ref)
    .bind(branch_id)
    .bind(submission_id)
    .bind(currency)
    .bind(amount)
    .bind(rate)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("the saved row {document_ref}: {e}"));
}

async fn save(db: &Devices, n: u32, amount: &str, rate: &str) -> String {
    let submission = format!("sub-exact-{n}");
    the_saved_row(
        &db.pool,
        MAIN,
        &format!("XC-20261007-{n:06}"),
        &submission,
        "USD",
        amount,
        rate,
    )
    .await;
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

/// What PostgreSQL holds, read back as text: every digit of the baht (#759: rounded
/// HALF_UP to 2 places) and of
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
        // #759 (R2): the product rounded HALF_UP to 2 places once.
        let exact = (Decimal::from_str(amount).unwrap() * Decimal::from_str(rate).unwrap())
            .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
        assert_eq!(Decimal::from_str(thb).unwrap(), exact);
        // the rest of the line is as before
        assert_eq!(line.detail["mode"], "EMPLOYEE_CONFIRMATION");
        assert_eq!(line.detail["currency"], "USD");
    }
    db.finish().await;
}
