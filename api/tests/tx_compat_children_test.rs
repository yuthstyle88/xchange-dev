//! Ticket #670 (TX-2) — the one writer of today, the compat save
//! (`SaveTransactionWorkflow` → `PgTransactionRepository::insert_in`), also writes the three child
//! rows from TX-2 on, with the wide row's own figures, and numbers from the organisation's counter.
//! Written before the implementation by the ticket author (claude/f03345fe). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §2 D-1/D-2 (one writer; the wide columns
//! stay, the children are written beside them), §3.3–§3.5, §8 item 2 (the same mapping as the
//! backfill: `settle_thb` = `paid` for BUY and `received` for SELL; the SLIP and RECEIPT files),
//! ticket §9 *"A compat save after TX-2 writes one row in each child table, with the same figures
//! as the wide row"*. Lane controller's ruling on #670 (2026-10-10): the counter key is
//! `document-<organization_id>-<mode>-<date>`.
//!
//! The harness is `save_common` (ticket T8b). Needs `TEST_DATABASE_URL`; nothing here skips.

mod save_common;

use save_common::*;

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";

/// The wide row and its children as one comparable text per exchange.
async fn joined(shop: &Shop, number: &str) -> String {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT concat_ws('|', \
            t.status, (t.completed_at = t.recorded_at)::text, \
            (l.shop_side = t.type)::text, (l.currency = t.code)::text, (l.quantity = t.amount)::text, \
            (l.unit_size = t.unit_size)::text, (l.rate_snapshot = t.rate)::text, \
            (l.reference_rate IS NOT DISTINCT FROM t.reference_rate)::text, \
            (l.thb_amount = t.principal)::text, (l.fee_thb = t.fee)::text, \
            l.settle_thb::text, l.line_no::text, coalesce(l.quote_line_id::text, 'NULL'), \
            (l.organization_id = t.organization_id)::text, \
            p.method, p.reference, p.account, coalesce(p.slip_file_key, 'NULL'), \
            (p.amount_thb = l.settle_thb)::text, (p.bank_fee_thb = t.bank_fee)::text, \
            (p.paid_at = t.paid_at)::text, p.payment_no::text, \
            r.number, r.revision::text, (r.issued_at = t.recorded_at)::text, \
            (r.document_ref = (SELECT file_key FROM transaction_files f \
                               WHERE f.transaction_id = t.id AND f.kind = 'RECEIPT'))::text) \
         FROM transactions t \
         JOIN transaction_lines l ON l.transaction_id = t.id \
         JOIN transaction_payments p ON p.transaction_id = t.id \
         JOIN receipts r ON r.transaction_id = t.id \
         WHERE t.document_ref = '{number}'"
    )))
    .fetch_one(&shop.pool)
    .await
    .unwrap_or_else(|e| panic!("{number} with exactly one line, payment and receipt: {e}"))
}

#[tokio::test]
async fn a_compat_save_writes_one_line_one_payment_and_one_receipt_with_the_wide_rows_figures() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let slip = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;

    // a cash BUY: the shop pays settle_thb = paid
    let buy = shop
        .save(
            MAIN,
            form("child-buy-0001-aaaaaaaa", &customer, &passport),
            at(0),
        )
        .await
        .expect("a cash BUY");
    // a SELL by transfer with a slip: the shop receives settle_thb = received
    let mut cmd = form("child-sell-0001-aaaaaaa", &customer, &passport);
    cmd.side = "SELL".to_owned();
    cmd.rate = "33".to_owned();
    cmd.amount = "250.5".to_owned();
    cmd.fee = "10".to_owned();
    cmd.bank_fee = "1.5".to_owned();
    cmd.method = "Bank Transfer".to_owned();
    cmd.payment_ref = "TRX-1".to_owned();
    cmd.slip = Some(slip.clone());
    cmd.reference_rate = Some("32.95".to_owned());
    let sell = shop.save(MAIN, cmd, at(1)).await.expect("a SELL");

    for table in [
        "transactions",
        "transaction_lines",
        "transaction_payments",
        "receipts",
    ] {
        assert_eq!(shop.count(table).await, 2, "{table}: one per save");
    }
    let t = "true";
    assert_eq!(
        joined(&shop, &buy.document_ref).await,
        [
            "COMPLETED",
            t,
            t,
            t,
            t,
            t,
            t,
            t,
            t,
            t,
            "3210.00",
            "1",
            "NULL",
            t,
            "Cash",
            "",
            "Cash drawer 1",
            "NULL",
            t,
            t,
            t,
            "1",
            &buy.document_ref,
            "1",
            t,
            t
        ]
        .join("|"),
        "the BUY"
    );
    assert_eq!(
        joined(&shop, &sell.document_ref).await,
        [
            "COMPLETED",
            t,
            t,
            t,
            t,
            t,
            t,
            t,
            t,
            t,
            "8276.50",
            "1",
            "NULL",
            t,
            "Bank Transfer",
            "TRX-1",
            "Cash drawer 1",
            &slip,
            t,
            t,
            t,
            "1",
            &sell.document_ref,
            "1",
            t,
            t
        ]
        .join("|"),
        "the SELL"
    );
    shop.finish().await;
}

#[tokio::test]
async fn a_compat_save_numbers_from_its_organisations_counter() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let saved = shop
        .save(
            MAIN,
            form("org-key-0001-aaaaaaaaa", &customer, &passport),
            at(0),
        )
        .await
        .expect("a save");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    let keys: Vec<(String, i64)> =
        sqlx::query_as("SELECT key, value FROM counters WHERE key LIKE 'document-%' ORDER BY key")
            .fetch_all(&shop.pool)
            .await
            .expect("read counters");
    assert_eq!(
        keys,
        vec![(format!("document-{ORG_1}-LIVE-20261006"), 1)],
        "D-4: the stored key names the organisation"
    );
    shop.finish().await;
}
