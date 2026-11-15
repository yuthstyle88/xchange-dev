//! GA-4 (xchange#6928): migration `20261227040000_ga4_cash_step_and_rule_checks.sql` against a real
//! PostgreSQL. Design: `docs/design/blueprint/owner-set-values.md` §5 (data migration), rows 5, 6, 8, 9.
//!
//! - `cash_step_thb` on `quote_lines` and `transaction_lines`, backfilled to 0.25 on every line that was
//!   rounded (§5 item 2) and `NULL` elsewhere;
//! - the `*_rounding_within_a_coin` CHECKs, re-added under the same names: no step means no rounding, a
//!   step bounds the rounding;
//! - the branch bp cap and the 0–5 minimum spread range leave the DB (the Owner's rules set them);
//! - `pricing_suggestion_settings.better_pct` may be `NULL` ("not set").
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Each test works in a schema of
//! its own, so a test can stop the migrations just before GA-4, insert rows, then apply it.

mod quote_common;

use quote_common::*;

/// The migration under test.
const GA4: i64 = 20261227040000;
const CHECK: &str = "23514";

/// A schema with every migration before GA-4 applied.
async fn before_ga4() -> TestDb {
    let db = TestDb::empty().await;
    db.migrate_through(Some(GA4 - 1)).await;
    db
}

/// The name of the constraint `sql` violates, or `"ok"` when it succeeds.
async fn refused_by(db: &TestDb, sql: &str) -> String {
    match sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(&db.pool)
        .await
    {
        Ok(_) => "ok".to_owned(),
        Err(e) => {
            let d = e
                .as_database_error()
                .unwrap_or_else(|| panic!("{sql}: not a database error: {e}"));
            d.constraint()
                .map(str::to_owned)
                .unwrap_or_else(|| panic!("{sql}: {e} names no constraint"))
        }
    }
}

/// A publication with one USD line on MAIN and a quote on it; answers (quote id, published line id).
async fn quote(db: &TestDb) -> (String, String) {
    let p = db.publish("MAIN", &[USD]).await;
    let line = p.line("USD").to_owned();
    let id = uuid::Uuid::new_v4().to_string();
    db.exec(&format!(
        "INSERT INTO quotes (id, organization_id, branch_id, customer_id, type, channel, status, \
             publication_id, expires_at, created_by, accepted_by_transaction_id, created_at, updated_at) \
         VALUES ('{id}', '{ORG_1}', 'MAIN', NULL, 'INDICATIVE', 'DESK', 'ACTIVE', '{}', \
             now() + interval '300 seconds', 'employee-1', NULL, now(), now())",
        p.id
    ))
    .await;
    (id, line)
}

/// A DEMO draft transaction on MAIN (no shift needed: `DRAFT` needs no `completed_at`); answers its id.
async fn transaction(db: &TestDb) -> String {
    let customer = unique("c");
    db.customer(&customer, "MAIN").await;
    let id = uuid::Uuid::new_v4().to_string();
    db.exec(&format!(
        "INSERT INTO transactions (id, branch_id, mode, status, submission_id, payload_hash, customer_id, \
             customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
             recorded_by, organization_id) \
         VALUES ('{id}', 'MAIN', 'DEMO', 'DRAFT', '{}', 'h', \
             '{customer}', 'Test Customer', 'TH', 'employee-1', '', '', current_date, now(), now(), \
             'employee-1', '{ORG_1}')",
        unique("sub")
    ))
    .await;
    id
}

/// A `quote_lines` INSERT: shop side, `thb_amount`, `settle_thb`, `cash_step_thb` (`None` leaves the
/// column out, as before GA-4). `fee_thb` is 0, so the rounding is `settle − thb` on a BUY and
/// `settle − thb` on a SELL alike.
fn quote_line(
    quote: &str,
    published: &str,
    line_no: i32,
    side: &str,
    thb: &str,
    settle: &str,
    step: Option<&str>,
) -> String {
    let (col, val) = match step {
        Some(s) => (", cash_step_thb", format!(", {s}")),
        None => ("", String::new()),
    };
    format!(
        "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, shop_side, currency, \
             denomination_group, quantity, unit_size, base_rate, final_rate, thb_amount, fee_thb, settle_thb, \
             line_no, created_at, updated_at{col}) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{quote}', '{published}', '{side}', 'USD', 'ALL', 10, 1, \
             32.10, 32.10, {thb}, 0, {settle}, {line_no}, now(), now(){val})"
    )
}

/// A `transaction_lines` INSERT, as [`quote_line`].
fn transaction_line(
    transaction: &str,
    line_no: i32,
    side: &str,
    thb: &str,
    settle: &str,
    step: Option<&str>,
) -> String {
    let (col, val) = match step {
        Some(s) => (", cash_step_thb", format!(", {s}")),
        None => ("", String::new()),
    };
    format!(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, quantity, \
             unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at{col}) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{transaction}', '{side}', 'USD', 10, 1, 32.10, {thb}, 0, \
             {settle}, {line_no}, now(), now(){val})"
    )
}

#[tokio::test]
async fn the_backfill_marks_every_rounded_line_with_the_old_quarter_step_and_no_other() {
    let db = before_ga4().await;
    let (q, published) = quote(&db).await;
    let t = transaction(&db).await;
    // a cash quote line rounded −0.13 (a SELL of 1,000.13 settled at 1,000.00) and a non-cash one
    db.exec(&quote_line(
        &q, &published, 1, "SELL", "1000.13", "1000.00", None,
    ))
    .await;
    db.exec(&quote_line(
        &q, &published, 2, "SELL", "500.13", "500.13", None,
    ))
    .await;
    // a cash transaction line rounded +0.12 (a BUY of 1,000.13 paid out at 1,000.25) and a non-cash one
    db.exec(&transaction_line(&t, 1, "BUY", "1000.13", "1000.25", None))
        .await;
    db.exec(&transaction_line(&t, 2, "BUY", "500.13", "500.13", None))
        .await;

    db.migrate_through(None).await;

    let steps = |table: &str| {
        format!(
            "SELECT string_agg(line_no || '=' || COALESCE(cash_step_thb::text, 'NULL'), ',' ORDER BY line_no) FROM {table}"
        )
    };
    assert_eq!(
        db.text(&steps("quote_lines")).await.as_deref(),
        Some("1=0.25,2=NULL"),
        "the rounded quote line holds 0.25, the zero-rounding one NULL"
    );
    assert_eq!(
        db.text(&steps("transaction_lines")).await.as_deref(),
        Some("1=0.25,2=NULL"),
        "the rounded transaction line holds 0.25, the zero-rounding one NULL"
    );
    db.finish().await;
}

/// The five cases of the new line CHECK, on one table. `insert(line_no, settle, step)` writes a SELL of
/// 1,000.50 (rounding = settle − 1,000.50).
async fn the_line_check_cases(
    db: &TestDb,
    name: &str,
    insert: impl Fn(i32, &str, Option<&str>) -> String,
) {
    let cases: [(&str, Option<&str>, &str); 5] = [
        ("1000.50", None, "ok"),         // no step, rounding 0
        ("1000.51", None, name),         // no step, rounding 0.01
        ("1000.10", Some("0.50"), "ok"), // step 0.50, rounding −0.40 (refused by the old `< 0.25`)
        ("1000.00", Some("0.50"), name), // step 0.50, rounding −0.50
        ("1000.50", Some("0"), "*"),     // a step of 0
    ];
    for (i, (settle, step, want)) in cases.into_iter().enumerate() {
        let got = refused_by(db, &insert(i as i32 + 1, settle, step)).await;
        if want == "*" {
            assert_ne!(got, "ok", "{name}: a step of 0 is refused");
        } else {
            assert_eq!(got, want, "{name}: settle {settle}, step {step:?}");
        }
    }
    // the step-0 refusal is a CHECK, not some other failure
    assert_eq!(
        db.outcome(&insert(9, "1000.50", Some("0"))).await,
        CHECK,
        "{name}: step 0"
    );
}

#[tokio::test]
async fn the_new_line_check_on_quote_lines_bounds_the_rounding_by_the_lines_own_step() {
    let db = TestDb::start().await;
    let (q, published) = quote(&db).await;
    the_line_check_cases(
        &db,
        "quote_lines_rounding_within_a_coin",
        |n, settle, step| quote_line(&q, &published, n, "SELL", "1000.50", settle, step),
    )
    .await;
    db.finish().await;
}

#[tokio::test]
async fn the_new_line_check_on_transaction_lines_bounds_the_rounding_by_the_lines_own_step() {
    let db = TestDb::start().await;
    let t = transaction(&db).await;
    the_line_check_cases(
        &db,
        "transaction_lines_rounding_within_a_coin",
        |n, settle, step| transaction_line(&t, n, "SELL", "1000.50", settle, step),
    )
    .await;
    db.finish().await;
}

#[tokio::test]
async fn the_branch_bp_cap_leaves_the_db() {
    let db = before_ga4().await;
    let row = |currency: &str, buy: &str, sell: &str| {
        format!(
            "INSERT INTO pricing_branch_policy (branch_id, currency_code, buy_bp, sell_bp, updated_by, updated_at) \
             VALUES ('MAIN', '{currency}', {buy}, {sell}, 'test', now())"
        )
    };
    assert_eq!(
        db.outcome(&row("USD", "450", "0")).await,
        CHECK,
        "before GA-4, 450 bp is outside -300..300"
    );
    db.migrate_through(None).await;
    assert_eq!(db.outcome(&row("USD", "450", "0")).await, "ok", "450 bp");
    assert_eq!(
        db.outcome(&row("JPY", "0", "NULL")).await,
        "ok",
        "sell_bp NULL"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_minimum_spread_range_leaves_the_db_but_not_below_zero() {
    let db = before_ga4().await;
    let policy = db.policy("MAIN", None, None).await;
    let currency_row = |currency: &str, pct: &str| {
        format!(
            "INSERT INTO rate_policy_currency_settings (policy_version_id, currency_code, min_spread_pct) \
             VALUES ('{policy}', '{currency}', {pct})"
        )
    };
    let suggestion_row = |scope: &str, pct: &str| {
        format!(
            "INSERT INTO pricing_suggestion_settings (branch_id, scope, better_pct, min_spread_pct, updated_by, updated_at) \
             VALUES ('MAIN', '{scope}', ARRAY[0.1,0.2,0.3,0.4]::numeric(9,4)[], {pct}, 'test', now())"
        )
    };
    assert_eq!(
        db.outcome(&currency_row("USD", "7.5")).await,
        CHECK,
        "before GA-4, 7.5 % is outside 0..5 (rate_policy_currency_settings)"
    );
    assert_eq!(
        db.outcome(&suggestion_row("USD", "7.5")).await,
        CHECK,
        "before GA-4, 7.5 % is outside 0..5 (pricing_suggestion_settings)"
    );
    db.migrate_through(None).await;
    assert_eq!(db.outcome(&currency_row("USD", "7.5")).await, "ok");
    assert_eq!(db.outcome(&currency_row("JPY", "-0.1")).await, CHECK);
    assert_eq!(db.outcome(&suggestion_row("USD", "7.5")).await, "ok");
    assert_eq!(db.outcome(&suggestion_row("JPY", "-0.1")).await, CHECK);
    db.finish().await;
}

#[tokio::test]
async fn better_pct_may_be_not_set_but_still_has_four_elements_when_set() {
    let db = TestDb::start().await;
    let row = |scope: &str, better: &str| {
        format!(
            "INSERT INTO pricing_suggestion_settings (branch_id, scope, better_pct, updated_by, updated_at) \
             VALUES ('MAIN', '{scope}', {better}, 'test', now())"
        )
    };
    assert_eq!(
        db.outcome(&row("*", "NULL")).await,
        "ok",
        "a '*' row with better_pct NULL"
    );
    assert_eq!(
        db.outcome(&row("USD", "ARRAY[0.1,0.2,0.3]::numeric(9,4)[]"))
            .await,
        CHECK,
        "three elements"
    );
    db.finish().await;
}
