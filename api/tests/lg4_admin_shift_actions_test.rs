//! Ticket #929 (LG-4) — the owner's direct shift actions on `/api/rpc` post through the ledger (design
//! `docs/design/blueprint/ledger-inventory.md` §4.4, §4.5; lane controller's rulings on #929: `adminOpenShift`
//! option (a) and `adminMoveStock` in commit 2, `adminCloseShift`'s difference in commit 3;
//! `api/src/application/compat/rpc.rs` `shift_action` joins §5).
//! Written before the implementation by the ticket author (claude/89bd61d9, rule A). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! What this test pins ([required]):
//! - `adminOpenShift` posts `SAFE(branch) → DRAWER(new shift)` of the opening, `source_type OPENING`, in the same
//!   transaction as the shift; a safe that cannot cover it is refused (400) and no shift opens.
//! - `adminMoveStock` IN posts `SAFE → DRAWER`, OUT posts `DRAWER → SAFE`, of the amount.
//! - `adminCloseShift` posts `counted − the ledger drawer` (never the payload's `expected`) to `VARIANCE(branch)`: short ⇒ `DRAWER → VARIANCE`, over ⇒
//!   `VARIANCE → DRAWER`. `VARIANCE` is read with SQL, so this file compiles before `CashPlace::Variance` exists.
//!
//! `Desk::open` funds MAIN's ledger SAFE with 10,000,000 of every register currency (`lg4_fund/mod.rs`); `EMPTY` is
//! a branch inserted here, not through `seed_branch`, so its safe holds nothing. Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use rpc_common::{App, Call, Cookies, Desk, MAIN, RPC, one_of_each_role, sign_in};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_cash::domain::posting::{CashPlace, CashPostings};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

const ORIGIN: &str = "http://desk.test";
const EMPTY: &str = "EMPTY";

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn request_id() -> String {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("00000000-0000-4000-8000-{n:012x}")
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'Empty')")
            .bind(EMPTY)
            .execute(&desk.db.pool)
            .await
            .expect("a branch whose safe holds nothing");
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room { desk, app, cookies }
    }

    async fn owner(&self, action: &str, payload: Value) -> (u16, String) {
        self.as_who("OWNER", action, payload).await
    }

    async fn as_who(&self, who: &str, action: &str, payload: Value) -> (u16, String) {
        let answer = self
            .app
            .send(Call::action(ORIGIN, RPC, action, payload).cookie(&self.cookies[who]))
            .await;
        (answer.status, answer.text.clone())
    }

    async fn open_shift(&self, branch: &str, opening: Value) -> String {
        let (status, text) = self
            .owner(
                "adminOpenShift",
                json!({ "branchId": branch, "requestId": request_id(), "opening": opening }),
            )
            .await;
        assert_eq!(status, 200, "adminOpenShift in {branch}: {text:.400}");
        self.open_shift_id(branch)
            .await
            .expect("the shift it opened")
    }

    async fn open_shift_id(&self, branch: &str) -> Option<String> {
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch)
            .fetch_optional(&self.desk.db.pool)
            .await
            .expect("read the open shift")
    }

    async fn at(&self, place: CashPlace, currency: &str) -> Decimal {
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        let b = LedgerCashPostings::new()
            .expected_in(&mut tx, &place)
            .await
            .expect("expected_in");
        tx.rollback().await.unwrap();
        Decimal::new(b.get(CurrencyCode::parse(currency).unwrap()).minor(), 2)
    }

    async fn variance(&self, branch: &str, currency: &str) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(
            "SELECT sum(e.quantity_signed) FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
             WHERE a.kind = 'VARIANCE' AND a.branch_id = $1 AND trim(a.currency) = $2",
        )
        .bind(branch)
        .bind(currency)
        .fetch_one(&self.desk.db.pool)
        .await
        .expect("read VARIANCE")
        .unwrap_or_default()
    }

    /// An exchange after the open that moves only the ledger drawer (TX-5's `post_in`): `shift_balances`, and so a
    /// v2 close's `expected`, does not see it.
    async fn exchange_into(&self, shift: &str, currency: &str, amount: &str) {
        use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
        use xc_cash::domain::posting::PostingRequest;
        let mut tx = self.desk.db.pool.begin().await.unwrap();
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Customer,
                    to: CashPlace::Drawer(shift.to_owned()),
                    currency: CurrencyCode::parse(currency).unwrap(),
                    quantity: xc_kernel::money::ForeignAmount::from_decimal(d(amount)),
                    source: SourceRef {
                        source_type: SourceType::Exchange,
                        source_id: request_id(),
                        source_key: "complete".to_string(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("an exchange into the drawer");
        tx.commit().await.unwrap();
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn safe(branch: &str) -> CashPlace {
    CashPlace::Safe(BranchId::new(branch.to_owned()))
}

#[tokio::test]
async fn admin_open_shift_moves_the_opening_from_the_safe_into_the_new_drawer() {
    let room = Room::open().await;
    let shift = room
        .open_shift(MAIN, json!({ "THB": 5000, "USD": 100 }))
        .await;
    assert_eq!(
        room.at(CashPlace::Drawer(shift.clone()), "THB").await,
        d("5000.00")
    );
    assert_eq!(room.at(CashPlace::Drawer(shift), "USD").await, d("100.00"));
    assert_eq!(
        room.at(safe(MAIN), "THB").await,
        d("9995000.00"),
        "the safe gave it"
    );
    assert_eq!(room.at(safe(MAIN), "USD").await, d("9999900.00"));
    let opening: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ledger_postings WHERE source_type = 'OPENING' AND branch_id = $1 \
         AND source_id NOT LIKE 'lg4-fund:%'",
    )
    .bind(MAIN)
    .fetch_one(&room.desk.db.pool)
    .await
    .unwrap();
    assert_eq!(opening, 1, "one OPENING posting for the opening");
    // Option A (issuecomment-6115055843): one posting, one entry pair (SAFE −, DRAWER +) per currency.
    let pairs: Vec<(String, i64)> = sqlx::query_as(
        "SELECT trim(e.currency), count(*) FROM ledger_entries e JOIN ledger_postings p ON p.id = e.posting_id \
         WHERE p.source_type = 'OPENING' AND p.branch_id = $1 AND p.source_id NOT LIKE 'lg4-fund:%' \
         GROUP BY 1 ORDER BY 1",
    )
    .bind(MAIN)
    .fetch_all(&room.desk.db.pool)
    .await
    .unwrap();
    assert_eq!(
        pairs,
        vec![("THB".to_owned(), 2), ("USD".to_owned(), 2)],
        "one entry pair per currency inside the one posting"
    );
    room.finish().await;
}

#[tokio::test]
async fn admin_open_shift_in_a_branch_whose_safe_cannot_cover_it_is_refused_and_opens_nothing() {
    let room = Room::open().await;
    let (status, text) = room
        .owner(
            "adminOpenShift",
            json!({ "branchId": EMPTY, "requestId": request_id(), "opening": { "THB": 5000 } }),
        )
        .await;
    assert_eq!(status, 400, "EMPTY's safe holds nothing: {text:.400}");
    assert_eq!(room.open_shift_id(EMPTY).await, None, "no shift opens");
    assert_eq!(room.at(safe(EMPTY), "THB").await, d("0.00"));
    room.finish().await;
}

#[tokio::test]
async fn admin_move_stock_in_and_out_moves_cash_between_safe_and_drawer() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN, json!({ "USD": 100 })).await;
    for (direction, amount) in [("IN", "250.25"), ("OUT", "50.00")] {
        let (status, text) = room
            .as_who(
                "FINANCE",
                "adminMoveStock",
                json!({ "requestId": request_id(), "shiftId": shift, "currency": "USD",
                        "direction": direction, "amount": amount, "reason": "lg4" }),
            )
            .await;
        assert_eq!(status, 200, "adminMoveStock {direction}: {text:.400}");
    }
    assert_eq!(
        room.at(CashPlace::Drawer(shift), "USD").await,
        d("300.25"),
        "100 opened + 250.25 in − 50 out"
    );
    assert_eq!(room.at(safe(MAIN), "USD").await, d("9999699.75"));
    room.finish().await;
}

#[tokio::test]
async fn admin_close_shift_short_puts_the_difference_on_variance() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN, json!({ "THB": 5000 })).await;
    let (status, text) = room
        .owner(
            "adminCloseShift",
            json!({ "requestId": request_id(), "shiftId": shift,
                    "counted": { "THB": 4900 }, "expected": { "THB": 5000 }, "note": "short" }),
        )
        .await;
    assert_eq!(status, 200, "adminCloseShift: {text:.400}");
    assert_eq!(room.variance(MAIN, "THB").await, d("100"), "THB 100 short");
    assert_eq!(room.at(CashPlace::Drawer(shift), "THB").await, d("4900.00"));
    room.finish().await;
}

#[tokio::test]
async fn admin_close_shift_over_takes_the_difference_out_of_variance() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN, json!({ "THB": 5000 })).await;
    let (status, text) = room
        .owner(
            "adminCloseShift",
            json!({ "requestId": request_id(), "shiftId": shift,
                    "counted": { "THB": 5100 }, "expected": { "THB": 5000 }, "note": "over" }),
        )
        .await;
    assert_eq!(status, 200, "adminCloseShift: {text:.400}");
    assert_eq!(room.variance(MAIN, "THB").await, d("-100"), "THB 100 over");
    assert_eq!(room.at(CashPlace::Drawer(shift), "THB").await, d("5100.00"));
    room.finish().await;
}

/// Gap 1 of the rule-A review (issuecomment-6113089577, ruled issuecomment-6115022197): the difference is
/// `counted − the ledger drawer`, not `counted − the payload's expected`. The payload's `expected` (5,000, what
/// `shift_balances` holds) is stale against the ledger drawer (5,500 after an exchange), so a close that trusted the
/// payload would post THB 400 out of VARIANCE instead of THB 100 into it.
#[tokio::test]
async fn admin_close_shift_takes_the_difference_from_the_ledger_drawer_not_the_payloads_expected() {
    let room = Room::open().await;
    let shift = room.open_shift(MAIN, json!({ "THB": 5000 })).await;
    room.exchange_into(&shift, "THB", "500.00").await;
    assert_eq!(
        room.at(CashPlace::Drawer(shift.clone()), "THB").await,
        d("5500.00")
    );
    let (status, text) = room
        .owner(
            "adminCloseShift",
            json!({ "requestId": request_id(), "shiftId": shift,
                    "counted": { "THB": 5400 }, "expected": { "THB": 5000 }, "note": "short" }),
        )
        .await;
    assert_eq!(status, 200, "adminCloseShift: {text:.400}");
    assert_eq!(
        room.variance(MAIN, "THB").await,
        d("100"),
        "THB 100 short of the ledger drawer's 5,500 (the payload's 5,000 would say 400 over)"
    );
    assert_eq!(room.at(CashPlace::Drawer(shift), "THB").await, d("5400.00"));
    room.finish().await;
}
