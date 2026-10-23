//! Ticket #1044 (LG-6.r5, `docs/design/blueprint/ledger-inventory.md` §4.2.1 and §10 row LG-6.r5, as amended by
//! design PR 1037): after the LG-4 cutover the branch safe is the ledger's `SAFE` account (design §4.4, §9
//! migration 2), so the compat snapshot's `operations.safe` is `CashPostings::expected_in(Safe(branch))`, never
//! the `ops_state.safe` blob. Written before the implementation by the ticket's test author (claude/d7aa64, taking
//! over from claude/89bd61d9). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Every case stores a *different* figure in `ops_state.safe` (raw SQL: the column stays until LG-6.m) from the one
//! the ledger holds, so a reader that still reads the blob prints the wrong number.
//!
//! Ticket §9 rows:
//! - Row 1 (a branch whose safe was backfilled at cutover): `"safe"` equals the `SAFE` account, in both
//!   `adminSnapshot` and `adminDeskConfig`; also for a branch that has a `SAFE` account and no `ops_state` row.
//! - Row 2 (a drawer→safe move after cutover): `"safe"` includes it.
//! - Row 3 (a branch with no `SAFE` account): `"safe"` is `null`, the value a NULL `ops_state.safe` gives today.
//! - Emptied safe (lane controller's ruling on #1044, 2026-10-15): a `SAFE` account that has never had a posting
//!   prints `null` (v2's "never counted in": `custody.json` steps 3 and 24 on `origin/main` `2282023c`); one that
//!   has had postings and nets to zero in every currency prints zeros, not `null` (v2's `{CUR: number}` over every
//!   register code, so the zeros of the currencies that had activity are among them).
//!
//! Ticket §8: `"safe"` keeps its JSON shape (v2's `{CUR: number}` over every register code, `balances_json`).
//!
//! Assumes only public API that exists on the base (`SnapshotBuilder::admin_snapshot` / `desk_config`,
//! `LedgerCashPostings`, `v2_json::balances_json`); it names no `safe` field of `CustodyState`, so it compiles
//! before and after the field goes.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod compat_common;
mod snapshot_common;

use rust_decimal::Decimal;
use serde_json::{Value, json};
use snapshot_common::{MAIN, Shop, branch, time};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money};
use xc_shift::domain::shift::value_objects::balances::Balances;
use xchange::application::compat::v2_json::balances_json;

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a register currency")
}

fn money(text: &str) -> Money {
    Money::parse(text).expect("an amount")
}

/// `"safe"` as v2 prints these balances: every register code, zero where none.
fn printed(pairs: &[(&str, &str)]) -> Value {
    let pairs: Vec<(CurrencyCode, ForeignAmount)> = pairs
        .iter()
        .map(|(c, a)| {
            (
                code(c),
                ForeignAmount::from_decimal(Decimal::new(money(a).try_minor().unwrap(), 2)),
            )
        })
        .collect();
    balances_json(&Balances::of(&pairs)).to_value()
}

/// One posting moving `amount` of `currency` from `from` to `to`.
async fn post(
    shop: &Shop,
    from: CashPlace,
    to: CashPlace,
    currency: &str,
    amount: &str,
    source_type: SourceType,
    source_id: &str,
) {
    let mut tx = shop.db.pool.begin().await.expect("begin");
    LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from,
                to,
                currency: code(currency),
                quantity: ForeignAmount::from_decimal(
                    amount.parse::<Decimal>().expect("an amount"),
                ),
                source: SourceRef {
                    source_type,
                    source_id: source_id.to_string(),
                    source_key: format!("lg6r5:{currency}"),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("post {source_id}: {e:?}"));
    tx.commit().await.expect("commit");
}

/// The cutover's backfill for `branch`: `OPENING` from BANK into `SAFE` (design §9 migration 2).
async fn backfill_safe(shop: &Shop, branch: &BranchId, currency: &str, amount: &str) {
    post(
        shop,
        CashPlace::Bank,
        CashPlace::Safe(branch.clone()),
        currency,
        amount,
        SourceType::Opening,
        &format!("lg6r5-backfill:{}:{currency}", branch.as_str()),
    )
    .await;
}

/// An OPEN shift of `branch` (a drawer needs its shift: `ledger_postings.rs` `branch_of`).
async fn open_shift(shop: &Shop, branch: &BranchId, id: &str) {
    let teller_id: uuid::Uuid = if let Some(id) = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT u.id FROM users u JOIN organizations o ON o.id = u.organization_id \
         WHERE o.code = 'XC' AND u.name = 'test' AND u.created_by = 'test' \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .fetch_optional(&shop.db.pool)
    .await
    .expect("look up teller user")
    {
        id
    } else {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT id, 'test', 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
             RETURNING id",
        )
        .fetch_one(&shop.db.pool)
        .await
        .expect("insert teller user")
    };
    let inserted = sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now(), 'test', $3, $4)",
    )
    .bind(id)
    .bind(branch.as_str())
    .bind(format!("open-{id}"))
    .bind(teller_id)
    .execute(&shop.db.pool)
    .await
    .expect("insert an OPEN shift")
    .rows_affected();
    assert_eq!(
        inserted, 1,
        "the OPEN shift needs the seeded `test` user as its teller"
    );
}

/// `operations.safe` of `branch` in `adminSnapshot` and in `adminDeskConfig`.
async fn safes(shop: &Shop, branch: &BranchId) -> (Value, Value) {
    let builder = shop.builder();
    let snapshot = builder
        .admin_snapshot(branch, "owner", time("2026-10-15T05:00:00.000Z"))
        .await
        .unwrap_or_else(|e| panic!("adminSnapshot of {}: {e:?}", branch.as_str()))
        .to_value();
    let desk = builder
        .desk_config(branch)
        .await
        .unwrap_or_else(|e| panic!("adminDeskConfig of {}: {e:?}", branch.as_str()))
        .to_value();
    (
        snapshot["operations"]["safe"].clone(),
        desk["operations"]["safe"].clone(),
    )
}

// ── §9 row 1 ────────────────────────────────────────────────────────────────

/// `ops_state.safe` is dropped by #1047; `operations.safe` comes from the ledger's `SAFE` account.
#[tokio::test]
async fn row1_the_snapshot_safe_is_the_safe_account_not_the_ops_state_blob() {
    let shop = Shop::open().await;
    let main = branch(MAIN);
    backfill_safe(&shop, &main, "THB", "12345.67").await;
    backfill_safe(&shop, &main, "USD", "250").await;

    let expected = printed(&[("THB", "12345.67"), ("USD", "250")]);
    let (snapshot, desk) = safes(&shop, &main).await;
    assert_eq!(
        snapshot, expected,
        "adminSnapshot operations.safe = SAFE(MAIN)"
    );
    assert_eq!(
        desk, expected,
        "adminDeskConfig operations.safe = SAFE(MAIN)"
    );

    shop.finish().await;
}

#[tokio::test]
async fn row1_a_branch_with_a_safe_account_and_no_ops_state_row_shows_the_account() {
    let shop = Shop::open().await;
    shop.db.seed_branch("NIGHT", "Night").await; // funds SAFE(NIGHT) with 10,000,000 of every code
    let night = branch("NIGHT");
    let codes: Vec<String> = sqlx::query_scalar("SELECT trim(code) FROM currencies ORDER BY 1")
        .fetch_all(&shop.db.pool)
        .await
        .expect("the currency register");
    let funded: Vec<(&str, &str)> = codes.iter().map(|c| (c.as_str(), "10000000")).collect();

    let expected = printed(&funded);
    let (snapshot, desk) = safes(&shop, &night).await;
    assert_eq!(
        snapshot, expected,
        "adminSnapshot operations.safe = SAFE(NIGHT)"
    );
    assert_eq!(
        desk, expected,
        "adminDeskConfig operations.safe = SAFE(NIGHT)"
    );

    shop.finish().await;
}

// ── §9 row 2 ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn row2_a_drawer_to_safe_move_after_cutover_is_in_the_snapshot_safe() {
    let shop = Shop::open().await;
    let main = branch(MAIN);
    backfill_safe(&shop, &main, "THB", "10000").await;

    // After the cutover: a shift opens with 3,000 from the safe, then moves 500 back.
    let shift = "lg6r5-shift";
    open_shift(&shop, &main, shift).await;
    post(
        &shop,
        CashPlace::Safe(main.clone()),
        CashPlace::Drawer(shift.to_string()),
        "THB",
        "3000",
        SourceType::Opening,
        "lg6r5-open",
    )
    .await;
    post(
        &shop,
        CashPlace::Drawer(shift.to_string()),
        CashPlace::Safe(main.clone()),
        "THB",
        "500",
        SourceType::Transfer,
        "lg6r5-move-out",
    )
    .await;

    let expected = printed(&[("THB", "7500")]);
    let (snapshot, desk) = safes(&shop, &main).await;
    assert_eq!(snapshot, expected, "adminSnapshot: 10000 − 3000 + 500");
    assert_eq!(desk, expected, "adminDeskConfig: 10000 − 3000 + 500");

    shop.finish().await;
}

// ── §9 row 3 ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn row3_a_branch_with_no_safe_account_prints_null() {
    let shop = Shop::open().await;
    let main = branch(MAIN);
    // No SAFE(MAIN) account in the ledger (`Shop::open` funds no safe for MAIN).
    let accounts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cash_accounts WHERE kind = 'SAFE' AND branch_id = 'MAIN'",
    )
    .fetch_one(&shop.db.pool)
    .await
    .expect("count SAFE accounts");
    assert_eq!(accounts, 0, "the case needs a branch with no SAFE account");

    let (snapshot, desk) = safes(&shop, &main).await;
    assert_eq!(
        snapshot,
        Value::Null,
        "adminSnapshot: no SAFE account → null"
    );
    assert_eq!(desk, Value::Null, "adminDeskConfig: no SAFE account → null");

    shop.finish().await;
}

// ── emptied safe (lane controller's ruling on #1044) ────────────────────────

#[tokio::test]
async fn emptied_a_safe_that_had_postings_and_nets_to_zero_prints_zeros_not_null() {
    let shop = Shop::open().await;
    let main = branch(MAIN);
    // SAFE(MAIN) is counted in, then everything leaves it again: it has postings, and nets to zero.
    backfill_safe(&shop, &main, "THB", "1000").await;
    backfill_safe(&shop, &main, "USD", "50").await;
    for (currency, amount) in [("THB", "1000"), ("USD", "50")] {
        post(
            &shop,
            CashPlace::Safe(main.clone()),
            CashPlace::Bank,
            currency,
            amount,
            SourceType::Transfer,
            &format!("lg6r5-empty:{currency}"),
        )
        .await;
    }

    let expected = printed(&[]);
    assert!(
        expected
            .as_object()
            .is_some_and(|codes| !codes.is_empty() && codes.values().all(|v| v == &json!(0))),
        "zeros over every register code: {expected}"
    );
    let (snapshot, desk) = safes(&shop, &main).await;
    assert_eq!(
        snapshot, expected,
        "adminSnapshot: an emptied SAFE prints zeros"
    );
    assert_eq!(
        desk, expected,
        "adminDeskConfig: an emptied SAFE prints zeros"
    );

    shop.finish().await;
}
