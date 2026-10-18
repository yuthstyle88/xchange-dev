//! Ticket #670 (TX-2) — unit_size and published_rate_line_id scoping and rate tests.
//!
//! 1. Scoping by branch_id: two organisations save on the same day (and thus share the same
//!    XC-YYYYMMDD-000001 document reference per D-4), and each transaction row keeps its own
//!    unit_size and published_rate_line_id (controller addendum issuecomment-6099329091).
//! 2. Write the resolved unit_size (100) when vouched by published rates. Re-expected for TX-6 #674 by
//!    the test-writer (decision A2 (owner, 2026-10-11; design quote-transaction.md §4.6.1, #674)): the unvouched DEMO save this item kept at main's unit_size 1 no
//!    longer exists; a DEMO save is quoted like a LIVE one and, with nothing published, is refused.

mod save_common;

use rust_decimal::Decimal;
use save_common::*;
use sqlx::Row;
use uuid::Uuid;

const ORG_2: &str = "00000000-0000-0000-0000-000000000002";

async fn publish_rate_direct(
    pool: &sqlx::PgPool,
    org: Uuid,
    branch: &str,
    code: &str,
    buy: &str,
    sell: &str,
    unit_size: Decimal,
) -> Uuid {
    let pub_id = Uuid::new_v4();
    let appr_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES ($1, $2, 'RATE_PUBLICATION', $3, $4, 'hash', '1', '{}'::jsonb, 'test', now(), 'APPROVED', $5)",
    )
    .bind(appr_id)
    .bind(org)
    .bind(branch)
    .bind(pub_id.to_string())
    .bind(format!("test_idem_{pub_id}"))
    .execute(pool)
    .await
    .expect("insert approval request");

    sqlx::query(
        "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, approved_request_id, draft_id, published_by) \
         VALUES ($1, $2, $3, 1, now(), $4, NULL, 'test')",
    )
    .bind(pub_id)
    .bind(org)
    .bind(branch)
    .bind(appr_id)
    .execute(pool)
    .await
    .expect("insert rate publication");

    sqlx::query(
        "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, denomination_group, value_tier, unit_size, buy_rate, sell_rate, source_snapshot) \
         VALUES ($1, $2, $3, $4, 'ALL', 'ALL', $5, $6::numeric, $7::numeric, '{\"source\":\"test\"}'::jsonb)",
    )
    .bind(line_id)
    .bind(org)
    .bind(pub_id)
    .bind(code)
    .bind(unit_size)
    .bind(buy)
    .bind(sell)
    .execute(pool)
    .await
    .expect("insert published rate line");

    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
        .bind(pub_id)
        .bind(branch)
        .execute(pool)
        .await
        .expect("update branch publication");

    line_id
}

#[tokio::test]
async fn two_organisations_saving_on_the_same_day_keep_their_own_unit_size_and_published_line() {
    let shop = Shop::start().await;
    let org2_uuid = Uuid::parse_str(ORG_2).unwrap();

    // Set up second organisation with branch B2
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1, 'ORG2', 'Second Org', 'Asia/Bangkok', now())",
    )
    .bind(org2_uuid)
    .execute(&shop.pool)
    .await
    .expect("insert org 2");

    sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ('B2', 'Branch 2', $1)")
        .bind(org2_uuid)
        .execute(&shop.pool)
        .await
        .expect("insert branch B2");

    // Automatically keep organization_id in sync with branch_id for multi-org test setup
    sqlx::query(
        "CREATE OR REPLACE FUNCTION t_set_org_from_branch() RETURNS trigger AS $$ \
         BEGIN \
             IF NEW.branch_id IS NOT NULL THEN \
                 SELECT organization_id INTO NEW.organization_id FROM branches WHERE id = NEW.branch_id; \
             END IF; \
             RETURN NEW; \
         END; \
         $$ LANGUAGE plpgsql;",
    )
    .execute(&shop.pool)
    .await
    .expect("create set_org trigger function");

    sqlx::query(
        "DO $$ \
         DECLARE \
             tbl text; \
         BEGIN \
             FOR tbl IN \
                 SELECT c1.table_name::text \
                 FROM information_schema.columns c1 \
                 JOIN information_schema.columns c2 ON c1.table_name = c2.table_name AND c1.table_schema = c2.table_schema \
                 WHERE c1.table_schema = current_schema() \
                   AND c1.column_name = 'branch_id' \
                   AND c2.column_name = 'organization_id' \
                   AND c1.table_name NOT IN ('branches', 'branch_versions') \
             LOOP \
                 EXECUTE format('DROP TRIGGER IF EXISTS t_set_org ON %I', tbl); \
                 EXECUTE format('CREATE TRIGGER t_set_org BEFORE INSERT ON %I FOR EACH ROW EXECUTE FUNCTION t_set_org_from_branch()', tbl); \
             END LOOP; \
         END $$;",
    )
    .execute(&shop.pool)
    .await
    .expect("attach set_org trigger to tables");

    // Seed owner user for Org 2
    sqlx::query(
        "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                    VALUES ($1, 'owner-org2', 'ACTIVE', 'test') RETURNING id, organization_id) \
         INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
         SELECT u.id, r.code, u.organization_id, NULL FROM u, (VALUES ('EXECUTIVE'), ('ADMIN')) AS r(code)",
    )
    .bind(org2_uuid)
    .execute(&shop.pool)
    .await
    .expect("seed org 2 owner user");

    // Prepare Org 1 (MAIN) with published JPY rate (per-1: 0.2195 / 0.2295, published unit_size = 100: 21.95 / 22.95)
    shop.set_rate(MAIN, "JPY", "0.2195", "0.2295").await;
    shop.open_shift(MAIN, &[("THB", "100000"), ("JPY", "100000")])
        .await;
    let customer_1 = shop.reserve(MAIN, person()).await;
    let passport_1 = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;

    // Prepare Org 2 (B2) with published USD rate (unit_size = 1)
    let b2_line_id = publish_rate_direct(
        &shop.pool,
        org2_uuid,
        "B2",
        "USD",
        "32.10",
        "33.00",
        Decimal::ONE,
    )
    .await;

    // CA-10 (#626, #2955): v2's `ShiftService::open` is deleted with the core. The same OPEN shift, written
    // straight into the tables in one branch transaction (setup only): opener "owner", whose user v2 resolved to
    // B2's organisation's owner (EXECUTIVE + ADMIN).
    let b2_shift = format!("SH-{}", uuid::Uuid::new_v4());
    let (id, request) = (b2_shift.clone(), unique("open-shift-b2").replace('_', "-"));
    shop.shifts
        .branch_tx()
        .clone()
        .run(&branch("B2"), move |tx| {
            Box::pin(async move {
                sqlx::query(
                    "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, \
                     teller_id) SELECT $1, 'B2', 'OPEN', $2, 'owner', '', $3, u.id FROM users u \
                     JOIN branches b ON b.id = 'B2' AND b.organization_id = u.organization_id \
                     WHERE u.legacy_employee_id IS NULL AND EXISTS (SELECT 1 FROM user_roles r \
                     WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
                     ORDER BY u.created_at LIMIT 1",
                )
                .bind(&id)
                .bind(at(-60))
                .bind(&request)
                .execute(&mut **tx)
                .await?;
                for (c, a) in [("THB", "100000"), ("USD", "10000")] {
                    sqlx::query(
                        "INSERT INTO shift_balances (shift_id, code, opening, expected) \
                         VALUES ($1, $2, $3::numeric, $3::numeric)",
                    )
                    .bind(&id)
                    .bind(c)
                    .bind(a)
                    .execute(&mut **tx)
                    .await?;
                }
                Ok::<(), pos_error::AppError>(())
            })
        })
        .await
        .expect("open shift in B2");
    // R1 (TX-6 #674, lane controller's ruling: compat saves post through the ledger): the same
    // opening in B2's ledger drawer.
    fund_ledger_drawer(
        &shop.pool,
        &b2_shift,
        &[("THB", "100000"), ("USD", "10000")],
    )
    .await;

    let customer_2 = shop.reserve("B2", person()).await;
    let passport_2 = shop.upload("B2", OWNER, "image/jpeg", JPEG).await;

    // Save 1: Org 1 (MAIN) sells 10000 JPY at 22.95 (rate per 100, unit_size 100)
    // client_rate is per-one = 0.2295
    let mut cmd_1 = form("org1-jpy-save-000001", &customer_1, &passport_1);
    cmd_1.side = "SELL".to_owned();
    cmd_1.currency = "JPY".to_owned();
    cmd_1.amount = "10000".to_owned();
    cmd_1.rate = "0.2295".to_owned();
    let save_1 = shop.save(MAIN, cmd_1, at(0)).await.expect("Org 1 save JPY");

    // Save 2: Org 2 (B2) sells 100 USD at 33.00 (rate per 1, unit_size 1)
    let mut cmd_2 = form("org2-usd-save-000001", &customer_2, &passport_2);
    cmd_2.side = "SELL".to_owned();
    cmd_2.currency = "USD".to_owned();
    cmd_2.amount = "100".to_owned();
    cmd_2.rate = "33".to_owned();
    let save_2 = shop
        .save("B2", cmd_2, at(10))
        .await
        .expect("Org 2 save USD");

    // Both organisations number from 1 per D-4 on the same day
    assert_eq!(save_1.document_ref, save_2.document_ref);

    // Read back Org 1's transaction
    let row_1 = sqlx::query(
        "SELECT ql.published_rate_line_id, l.unit_size, l.rate_snapshot AS rate FROM transaction_lines l JOIN transactions t ON t.id = l.transaction_id LEFT JOIN quote_lines ql ON ql.id = l.quote_line_id WHERE t.document_ref = $1 AND t.branch_id = $2 AND l.line_no = 1",
    )
    .bind(&save_1.document_ref)
    .bind(MAIN)
    .fetch_one(&shop.pool)
    .await
    .expect("fetch Org 1 row");

    let line_id_1: Option<Uuid> = row_1.get("published_rate_line_id");
    let unit_size_1: Decimal = row_1.get("unit_size");
    let rate_1: Decimal = row_1.get("rate");

    // Read back Org 2's transaction
    let row_2 = sqlx::query(
        "SELECT ql.published_rate_line_id, l.unit_size, l.rate_snapshot AS rate FROM transaction_lines l JOIN transactions t ON t.id = l.transaction_id LEFT JOIN quote_lines ql ON ql.id = l.quote_line_id WHERE t.document_ref = $1 AND t.branch_id = $2 AND l.line_no = 1",
    )
    .bind(&save_2.document_ref)
    .bind("B2")
    .fetch_one(&shop.pool)
    .await
    .expect("fetch Org 2 row");

    let line_id_2: Option<Uuid> = row_2.get("published_rate_line_id");
    let unit_size_2: Decimal = row_2.get("unit_size");
    let rate_2: Decimal = row_2.get("rate");

    // Assert that each row kept its own unit_size and published_rate_line_id
    assert!(line_id_1.is_some(), "Org 1 has published rate line");
    assert_eq!(line_id_2, Some(b2_line_id), "Org 2 kept B2 rate line");
    assert_ne!(line_id_1, line_id_2, "different rate lines across branches");
    assert_eq!(
        unit_size_1,
        Decimal::from(100),
        "Org 1 kept JPY unit_size 100"
    );
    assert_eq!(unit_size_2, Decimal::from(1), "Org 2 kept USD unit_size 1");
    assert_eq!(rate_1, Decimal::from_str_exact("22.95").unwrap());
    assert_eq!(rate_2, Decimal::from_str_exact("33").unwrap());

    // Also check child transaction_lines
    let line_row_1 = sqlx::query(
        "SELECT unit_size FROM transaction_lines WHERE transaction_id = (SELECT id FROM transactions WHERE document_ref = $1 AND branch_id = $2)",
    )
    .bind(&save_1.document_ref)
    .bind(MAIN)
    .fetch_one(&shop.pool)
    .await
    .expect("fetch Org 1 line");
    let line_unit_1: Decimal = line_row_1.get("unit_size");
    assert_eq!(line_unit_1, Decimal::from(100));

    let line_row_2 = sqlx::query(
        "SELECT unit_size FROM transaction_lines WHERE transaction_id = (SELECT id FROM transactions WHERE document_ref = $1 AND branch_id = $2)",
    )
    .bind(&save_2.document_ref)
    .bind("B2")
    .fetch_one(&shop.pool)
    .await
    .expect("fetch Org 2 line");
    let line_unit_2: Decimal = line_row_2.get("unit_size");
    assert_eq!(line_unit_2, Decimal::from(1));
}

#[tokio::test]
async fn a_demo_jpy_save_with_nothing_published_is_refused_while_vouched_records_resolved_unit_size()
 {
    let shop = Shop::start().await;
    shop.open_shift(MAIN, &[("THB", "1000000"), ("JPY", "1000000")])
        .await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;

    // 1. Re-expected for TX-6 by the test-writer, decision A2 (owner, 2026-10-11; design quote-transaction.md §4.6.1, #674): a DEMO save is quoted like a LIVE one,
    //    so a DEMO JPY save with no published JPY line is refused `rateUnavailable` and writes nothing
    //    (v2 and today's save stored it unvouched, unit_size 1).
    let demo_person = ReserveCustomer {
        mode: "DEMO".to_owned(),
        ..person()
    };
    let demo_customer = shop.reserve(MAIN, demo_person.clone()).await;
    let mut demo_cmd = form("demo-jpy-save-000001", &demo_customer, &passport);
    demo_cmd.customer = demo_person;
    demo_cmd.side = "BUY".to_owned();
    demo_cmd.currency = "JPY".to_owned();
    demo_cmd.amount = "10000".to_owned();
    demo_cmd.rate = "0.2195".to_owned();

    assert_eq!(
        refusal(shop.save(MAIN, demo_cmd, at(0)).await),
        "rateUnavailable: no published line",
        "decision A2: a DEMO save with nothing published is refused"
    );
    let demo_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM transactions WHERE branch_id = $1 AND submission_id = 'demo-jpy-save-000001'",
    )
    .bind(MAIN)
    .fetch_one(&shop.pool)
    .await
    .expect("count demo rows");
    assert_eq!(demo_rows, 0, "the refused DEMO save wrote no exchange");

    // 2. LIVE vouched save of JPY: published rate line vouches it, so rate is per-100 (21.95) and unit_size is 100
    shop.set_rate(MAIN, "JPY", "0.2195", "0.2295").await;
    let live_customer = shop.reserve(MAIN, person()).await;
    let mut live_cmd = form("live-jpy-save-000001", &live_customer, &passport);
    live_cmd.side = "BUY".to_owned();
    live_cmd.currency = "JPY".to_owned();
    live_cmd.amount = "10000".to_owned();
    live_cmd.rate = "0.2195".to_owned(); // client_rate per-one (21.95 / 100)

    let live_saved = shop
        .save(MAIN, live_cmd, at(10))
        .await
        .expect("live vouched JPY save");

    let live_row = sqlx::query(
        "SELECT l.rate_snapshot AS rate, l.unit_size, ql.published_rate_line_id FROM transaction_lines l JOIN transactions t ON t.id = l.transaction_id LEFT JOIN quote_lines ql ON ql.id = l.quote_line_id WHERE t.document_ref = $1 AND t.branch_id = $2 AND l.line_no = 1",
    )
    .bind(&live_saved.document_ref)
    .bind(MAIN)
    .fetch_one(&shop.pool)
    .await
    .expect("fetch live row");

    let live_rate: Decimal = live_row.get("rate");
    let live_unit_size: Decimal = live_row.get("unit_size");
    let live_line_id: Option<Uuid> = live_row.get("published_rate_line_id");

    assert_eq!(
        live_rate,
        Decimal::from_str_exact("21.95").unwrap(),
        "vouched JPY stores per-100 rate"
    );
    assert_eq!(
        live_unit_size,
        Decimal::from(100),
        "vouched JPY stores resolved unit_size 100"
    );
    assert!(
        live_line_id.is_some(),
        "vouched JPY has published rate line id"
    );

    // Also check child transaction_lines for live
    let live_line_row = sqlx::query(
        "SELECT unit_size, rate_snapshot FROM transaction_lines WHERE transaction_id = (SELECT id FROM transactions WHERE document_ref = $1 AND branch_id = $2)",
    )
    .bind(&live_saved.document_ref)
    .bind(MAIN)
    .fetch_one(&shop.pool)
    .await
    .expect("fetch live line row");
    let live_line_unit: Decimal = live_line_row.get("unit_size");
    let live_line_rate: Decimal = live_line_row.get("rate_snapshot");
    assert_eq!(live_line_unit, Decimal::from(100));
    assert_eq!(live_line_rate, Decimal::from_str_exact("21.95").unwrap());
}
