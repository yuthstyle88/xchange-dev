//! CA-6 (#621 Part 2): the kept drawer↔safe and external moves on the same five-state core: `SafeMove`
//! (DRAWER_TO_SAFE, SAFE_TO_DRAWER) and `ExternalMove` (EXTERNAL_IN, EXTERNAL_OUT), one file each behind
//! `Arc<dyn TransferEnds>` (design `docs/design/blueprint/cash-shift-transfer.md` §2 C5, §3.2, §4.5, §4.6, §9.1 CA-6;
//! the money flow of `custody_machine.rs:420-634`, restated).
//!
//! Written before the implementation by claude/adc848e6, on the lane controller's order. Do not edit, delete or weaken
//! these tests; add your own in another file.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.
//!
//! [required] by these tests: everything `transfers_test.rs` and `transfer_flow_test.rs` list, plus:
//! - `TransferKinds::standard()` holds all five kinds.
//! - Ends and postings per kind (one `PostingRequest` per currency, `source` = the transfer id):
//!
//!   | kind | ends `to_branch / from_shift / to_shift` | dispatch | receipt (counted notes) |
//!   |---|---|---|---|
//!   | DRAWER_TO_SAFE | NULL / set / NULL | `Drawer(from_shift)` → `InTransit` | `InTransit` → `Safe(from_branch)` |
//!   | SAFE_TO_DRAWER | NULL / NULL / set | `Safe(from_branch)` → `InTransit` | `InTransit` → `Drawer(to_shift)` |
//!   | EXTERNAL_IN | NULL / NULL / NULL | `Outside` → `InTransit` | `InTransit` → `Safe(from_branch)` |
//!   | EXTERNAL_OUT | NULL / NULL / NULL | `Safe(from_branch)` → `InTransit` | `InTransit` → `Outside` |
//!
//! - Who (§3.2): create and dispatch need `CashManage` on the branch (BRANCH_MANAGER or FINANCE). The receiver of a
//!   drawer is **the teller whose shift it is** (`shifts.teller_id`); of the safe, a holder of `CashManage` on the
//!   branch; of external money, FINANCE (`CashManage`).
//! - Interim postings (ruling 2): Drawer = `shift_balances.expected` of the shift plus one `stock_movements` row
//!   (`direction` OUT when the drawer pays out, IN when it takes in; `amount` the currency total; `request_id` = the
//!   transfer id). Safe = `ops_state.safe`. `InTransit` and `Outside` are not balances.
//! - A dispatch beyond the drawer's or the safe's expected is `TransferError::StockInsufficient` and moves nothing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::posting::{CashPlace, PostingRequest};
use xc_cash::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferError, TransferKind, TransferLine,
    TransferStatus,
};
use xc_cash::domain::transfer::ends::TransferEnds;
use xc_cash::domain::transfer::kinds::TransferKinds;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

// ── the database ────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
}

impl Db {
    async fn fresh() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("ca6");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(dir.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        Db { pool }
    }

    async fn branch(&self, label: &str, safe_usd_minor: i64) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO ops_state (branch_id, enabled, phase, safe) VALUES ($1, true, 'OPEN', jsonb_build_object('USD', $2::bigint))",
        )
        .bind(&id)
        .bind(safe_usd_minor)
        .execute(&self.pool)
        .await
        .expect("the branch's safe");
        BranchId::new(id)
    }

    /// A user holding `roles`, each on a branch (`None` = organisation-wide), and their `Actor`.
    async fn person(&self, name: &str, roles: &[(Role, Option<&BranchId>)]) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        for (role, branch) in roles {
            sqlx::query(
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) VALUES ($1::uuid, $2, $3::uuid, $4)",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
            .bind(branch.map(|b| b.as_str().to_owned()))
            .execute(&self.pool)
            .await
            .expect("give the user a role");
        }
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            roles
                .iter()
                .map(|(r, b)| ScopedRole::new(*r, b.cloned(), from, None))
                .collect(),
            None,
        )
    }

    async fn usd(&self, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = $1::numeric",
        )
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the USD {face} face value (A7): {e}"))
    }

    /// `ops_state.safe` USD of a branch, in minor units (0 when absent).
    async fn safe_usd(&self, branch: &BranchId) -> i64 {
        sqlx::query_scalar(
            "SELECT coalesce((safe->>'USD')::bigint, 0) FROM ops_state WHERE branch_id = $1",
        )
        .bind(branch.as_str())
        .fetch_one(&self.pool)
        .await
        .expect("read the safe")
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    fn flow(&self) -> TransferFlow {
        TransferFlow::pg(self.pool.clone())
    }

    /// A teller's shift opened by the count path (#619): `usd_50s` × USD 50 in the drawer.
    async fn drawer(
        &self,
        branch: &BranchId,
        teller: &Actor,
        manager: &Actor,
        usd_50s: i32,
    ) -> String {
        let counts = CashCountService::pg(self.pool.clone());
        let lines = vec![CountLine::new("USD", self.usd("50").await, usd_50s)];
        let cmd = SubmitCount {
            branch: branch.clone(),
            shift_id: None,
            kind: CountKind::Open,
            lines,
            note: None,
            idempotency_key: unique("count"),
        };
        let count = counts
            .submit(teller, cmd, at(0))
            .await
            .expect("the teller counts");
        let verified = counts
            .verify(manager, &count.id, at(5))
            .await
            .expect("a BRANCH_MANAGER verifies");
        verified.shift_id.expect("the shift it opened")
    }

    async fn drawer_usd(&self, shift: &str) -> Decimal {
        sqlx::query_scalar(
            "SELECT expected FROM shift_balances WHERE shift_id = $1 AND code = 'USD'",
        )
        .bind(shift)
        .fetch_one(&self.pool)
        .await
        .expect("the drawer's USD")
    }

    /// `(direction, amount)` of the `stock_movements` rows a transfer wrote on a shift.
    async fn movements(&self, shift: &str, transfer: &str) -> Vec<(String, Decimal)> {
        sqlx::query_as(
            "SELECT direction, amount FROM stock_movements WHERE shift_id = $1 AND request_id = $2 ORDER BY seq",
        )
        .bind(shift)
        .bind(transfer)
        .fetch_all(&self.pool)
        .await
        .expect("read stock_movements")
    }
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn refused_with<T: std::fmt::Debug>(result: Result<T, AppError>, expected: TransferError) {
    match result {
        Err(AppError::Domain(sentence)) => assert_eq!(sentence, expected.to_string()),
        other => panic!(
            "expected AppError::Domain({:?}), got {other:?}",
            expected.to_string()
        ),
    }
}

// ── pure: the registry and each kind's ends and postings ───────────────────

fn ends(kind: TransferKind) -> Arc<dyn TransferEnds> {
    TransferKinds::standard()
        .get(kind)
        .unwrap_or_else(|| panic!("{kind:?} is registered"))
}

fn move_of(
    kind: TransferKind,
    from_shift: Option<&str>,
    to_shift: Option<&str>,
    sent: i32,
    received: Option<i32>,
) -> CashTransfer {
    CashTransfer {
        id: "m-1".to_owned(),
        kind,
        from_branch: BranchId::new("B1"),
        to_branch: None,
        from_shift: from_shift.map(str::to_owned),
        to_shift: to_shift.map(str::to_owned),
        status: TransferStatus::Approved,
        created_by: "creator".to_owned(),
        lines: vec![TransferLine {
            currency: usd(),
            denomination_id: 1,
            face_value: Money::parse("50").unwrap(),
            qty_sent: sent,
            qty_received: received,
        }],
    }
}

fn flat(reqs: &[PostingRequest]) -> HashSet<(String, String, String, String)> {
    reqs.iter()
        .map(|r| {
            (
                format!("{:?}", r.from),
                format!("{:?}", r.to),
                r.currency.as_str().to_owned(),
                format!("{:.2}", r.quantity.value()),
            )
        })
        .collect()
}

fn one(from: CashPlace, to: CashPlace, amount: &str) -> HashSet<(String, String, String, String)> {
    HashSet::from([(
        format!("{from:?}"),
        format!("{to:?}"),
        "USD".to_owned(),
        Money::parse(amount).unwrap().to_string(),
    )])
}

#[test]
fn the_standard_kinds_are_all_five_each_keyed_by_its_own_kind() {
    let kinds = TransferKinds::standard();
    assert_eq!(
        kinds.kinds(),
        vec![
            TransferKind::Branch,
            TransferKind::DrawerToSafe,
            TransferKind::ExternalIn,
            TransferKind::ExternalOut,
            TransferKind::SafeToDrawer,
        ],
        "sorted by as_str"
    );
    for k in kinds.kinds() {
        assert_eq!(kinds.get(k).unwrap().kind(), k);
    }
}

#[test]
fn each_moves_postings_follow_the_table() {
    let transit = || CashPlace::InTransit("m-1".to_owned());
    let safe = || CashPlace::Safe(BranchId::new("B1"));
    let drawer = |s: &str| CashPlace::Drawer(s.to_owned());
    // 4 × 50 sent, 3 counted
    let cases = [
        (
            TransferKind::DrawerToSafe,
            Some("s-1"),
            None,
            drawer("s-1"),
            transit(),
            transit(),
            safe(),
        ),
        (
            TransferKind::SafeToDrawer,
            None,
            Some("s-2"),
            safe(),
            transit(),
            transit(),
            drawer("s-2"),
        ),
        (
            TransferKind::ExternalIn,
            None,
            None,
            CashPlace::Bank,
            transit(),
            transit(),
            safe(),
        ),
        (
            TransferKind::ExternalOut,
            None,
            None,
            safe(),
            transit(),
            transit(),
            CashPlace::Bank,
        ),
    ];
    for (kind, from_shift, to_shift, d_from, d_to, r_from, r_to) in cases {
        let t = move_of(kind, from_shift, to_shift, 4, Some(3));
        assert_eq!(
            flat(&ends(kind).on_dispatch(&t)),
            one(d_from, d_to, "200"),
            "{kind:?} dispatch"
        );
        assert_eq!(
            flat(&ends(kind).on_receive(&t)),
            one(r_from, r_to, "150"),
            "{kind:?} receipt"
        );
    }
}

#[test]
fn each_move_refuses_the_wrong_ends() {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let bm = Actor::new(
        "bm",
        ORG,
        vec![ScopedRole::new(
            Role::BranchManager,
            Some(BranchId::new("B1")),
            from,
            None,
        )],
        None,
    );
    let new = |kind, to_branch: Option<&str>, from_shift: Option<&str>, to_shift: Option<&str>| {
        NewTransfer {
            kind,
            from_branch: BranchId::new("B1"),
            to_branch: to_branch.map(BranchId::new),
            from_shift: from_shift.map(str::to_owned),
            to_shift: to_shift.map(str::to_owned),
            lines: vec![NewTransferLine {
                currency: usd(),
                denomination_id: 1,
                qty: 1,
            }],
            note: None,
            idempotency_key: "k".to_owned(),
        }
    };
    let right = [
        new(TransferKind::DrawerToSafe, None, Some("s-1"), None),
        new(TransferKind::SafeToDrawer, None, None, Some("s-2")),
        new(TransferKind::ExternalIn, None, None, None),
        new(TransferKind::ExternalOut, None, None, None),
    ];
    for t in &right {
        assert_eq!(ends(t.kind).check_create(&bm, t), Ok(()), "{:?}", t.kind);
    }
    let wrong = [
        new(TransferKind::DrawerToSafe, None, None, None),
        new(TransferKind::DrawerToSafe, Some("B2"), Some("s-1"), None),
        new(TransferKind::SafeToDrawer, None, None, None),
        new(TransferKind::SafeToDrawer, None, Some("s-1"), Some("s-2")),
        new(TransferKind::ExternalIn, Some("B2"), None, None),
        new(TransferKind::ExternalIn, None, Some("s-1"), None),
        new(TransferKind::ExternalOut, None, None, Some("s-2")),
    ];
    for t in &wrong {
        assert!(
            ends(t.kind).check_create(&bm, t).is_err(),
            "{:?} {:?}/{:?}/{:?}",
            t.kind,
            t.to_branch,
            t.from_shift,
            t.to_shift
        );
    }
}

#[test]
fn a_teller_creates_no_move() {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let teller = Actor::new(
        "t",
        ORG,
        vec![ScopedRole::new(
            Role::Teller,
            Some(BranchId::new("B1")),
            from,
            None,
        )],
        None,
    );
    let t = NewTransfer {
        kind: TransferKind::SafeToDrawer,
        from_branch: BranchId::new("B1"),
        to_branch: None,
        from_shift: None,
        to_shift: Some("s-2".to_owned()),
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: 1,
            qty: 1,
        }],
        note: None,
        idempotency_key: "k".to_owned(),
    };
    assert!(
        ends(TransferKind::SafeToDrawer)
            .check_create(&teller, &t)
            .is_err()
    );
}

// ── over the tables ───────────────────────────────────────────────────────

struct Shop {
    b1: BranchId,
    manager: Actor,
    finance: Actor,
    exec: Actor,
    teller: Actor,
    other_teller: Actor,
}

async fn shop(db: &Db) -> Shop {
    let b1 = db.branch("b1", 50_000).await; // USD 500.00 in the safe
    let manager = db.person("Krit", &[(Role::BranchManager, Some(&b1))]).await;
    let finance = db.person("Pim", &[(Role::Finance, Some(&b1))]).await;
    let exec = db.person("Exec", &[(Role::Executive, None)]).await;
    let teller = db.person("Ploy", &[(Role::Teller, Some(&b1))]).await;
    let other_teller = db.person("Mew", &[(Role::Teller, Some(&b1))]).await;
    Shop {
        b1,
        manager,
        finance,
        exec,
        teller,
        other_teller,
    }
}

/// A move of `notes` × USD 50, created by `creator`, submitted, approved by EXECUTIVE and dispatched.
async fn dispatched(
    db: &Db,
    s: &Shop,
    kind: TransferKind,
    from_shift: Option<&str>,
    to_shift: Option<&str>,
    notes: i32,
) -> Result<CashTransfer, AppError> {
    let flow = db.flow();
    let cmd = NewTransfer {
        kind,
        from_branch: s.b1.clone(),
        to_branch: None,
        from_shift: from_shift.map(str::to_owned),
        to_shift: to_shift.map(str::to_owned),
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: db.usd("50").await,
            qty: notes,
        }],
        note: None,
        idempotency_key: unique("m"),
    };
    let t = flow
        .create(&s.finance, cmd, at(10))
        .await
        .expect("FINANCE creates the move");
    flow.submit(&s.finance, &t.id, at(11))
        .await
        .expect("submit");
    flow.decide(&s.exec, &t.id, Outcome::Approved, "ok", at(12))
        .await
        .expect("EXECUTIVE approves");
    flow.dispatch(&s.finance, &t.id, at(13)).await
}

async fn counted(db: &Db, notes: i32) -> Vec<NewTransferLine> {
    vec![NewTransferLine {
        currency: usd(),
        denomination_id: db.usd("50").await,
        qty: notes,
    }]
}

#[tokio::test]
async fn drawer_to_safe_takes_the_notes_out_of_the_drawer_at_dispatch_and_into_the_safe_at_receipt()
{
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let shift = db.drawer(&s.b1, &s.teller, &s.manager, 6).await; // USD 300 in the drawer
    let t = dispatched(&db, &s, TransferKind::DrawerToSafe, Some(&shift), None, 4)
        .await
        .expect("dispatch");
    assert_eq!(
        db.drawer_usd(&shift).await,
        d("100"),
        "USD 200 left the drawer"
    );
    assert_eq!(
        db.movements(&shift, &t.id).await,
        vec![("OUT".to_owned(), d("200"))]
    );
    assert_eq!(
        db.safe_usd(&s.b1).await,
        50_000,
        "in transit, not yet in the safe"
    );

    assert!(
        db.flow()
            .receive(&s.teller, &t.id, counted(&db, 4).await, at(20))
            .await
            .is_err(),
        "a teller does not receive into the safe"
    );
    let t = db
        .flow()
        .receive(&s.manager, &t.id, counted(&db, 4).await, at(21))
        .await
        .expect("BRANCH_MANAGER receives into the safe");
    assert_eq!(t.status, TransferStatus::Received);
    assert_eq!(db.safe_usd(&s.b1).await, 70_000);
}

#[tokio::test]
async fn a_move_out_of_a_drawer_beyond_its_expected_is_refused_and_nothing_moves() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let shift = db.drawer(&s.b1, &s.teller, &s.manager, 2).await; // USD 100
    refused_with(
        dispatched(&db, &s, TransferKind::DrawerToSafe, Some(&shift), None, 3).await,
        TransferError::StockInsufficient,
    );
    assert_eq!(db.drawer_usd(&shift).await, d("100"));
    assert_eq!(
        db.scalar(&format!(
            "SELECT count(*) FROM stock_movements WHERE shift_id = '{shift}'"
        ))
        .await,
        0
    );
}

#[tokio::test]
async fn safe_to_drawer_takes_from_the_safe_and_only_the_drawers_own_teller_receives_it() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let shift = db.drawer(&s.b1, &s.teller, &s.manager, 1).await; // USD 50
    let t = dispatched(&db, &s, TransferKind::SafeToDrawer, None, Some(&shift), 4)
        .await
        .expect("dispatch");
    assert_eq!(db.safe_usd(&s.b1).await, 30_000, "USD 200 left the safe");
    assert_eq!(
        db.drawer_usd(&shift).await,
        d("50"),
        "not in the drawer until received"
    );

    for who in [&s.other_teller, &s.manager] {
        assert!(
            db.flow()
                .receive(who, &t.id, counted(&db, 4).await, at(20))
                .await
                .is_err(),
            "only the shift's teller"
        );
    }
    db.flow()
        .receive(&s.teller, &t.id, counted(&db, 4).await, at(21))
        .await
        .expect("the drawer's teller receives");
    assert_eq!(db.drawer_usd(&shift).await, d("250"));
    assert_eq!(
        db.movements(&shift, &t.id).await,
        vec![("IN".to_owned(), d("200"))]
    );
}

#[tokio::test]
async fn a_move_out_of_the_safe_beyond_its_expected_is_refused() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    refused_with(
        dispatched(&db, &s, TransferKind::ExternalOut, None, None, 11).await, // USD 550 > 500
        TransferError::StockInsufficient,
    );
    assert_eq!(db.safe_usd(&s.b1).await, 50_000);
}

#[tokio::test]
async fn external_in_moves_nothing_at_dispatch_and_fills_the_safe_at_receipt_by_finance() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let t = dispatched(&db, &s, TransferKind::ExternalIn, None, None, 4)
        .await
        .expect("dispatch");
    assert_eq!(
        db.safe_usd(&s.b1).await,
        50_000,
        "outside money is not a balance"
    );
    assert!(
        db.flow()
            .receive(&s.teller, &t.id, counted(&db, 4).await, at(20))
            .await
            .is_err()
    );
    let other_finance = db.person("Ann", &[(Role::Finance, Some(&s.b1))]).await;
    db.flow()
        .receive(&other_finance, &t.id, counted(&db, 4).await, at(21))
        .await
        .expect("FINANCE receives");
    assert_eq!(db.safe_usd(&s.b1).await, 70_000);
}

#[tokio::test]
async fn external_out_empties_the_safe_at_dispatch_and_moves_nothing_at_receipt() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let t = dispatched(&db, &s, TransferKind::ExternalOut, None, None, 4)
        .await
        .expect("dispatch");
    assert_eq!(db.safe_usd(&s.b1).await, 30_000);
    let other_finance = db.person("Ann", &[(Role::Finance, Some(&s.b1))]).await;
    db.flow()
        .receive(&other_finance, &t.id, counted(&db, 4).await, at(21))
        .await
        .expect("FINANCE confirms the hand-over");
    assert_eq!(db.safe_usd(&s.b1).await, 30_000, "outside is not a balance");
}

#[tokio::test]
async fn a_short_receipt_of_a_move_writes_an_exception_on_its_branch() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let t = dispatched(&db, &s, TransferKind::ExternalOut, None, None, 4)
        .await
        .expect("dispatch");
    let other_finance = db.person("Ann", &[(Role::Finance, Some(&s.b1))]).await;
    db.flow()
        .receive(&other_finance, &t.id, counted(&db, 3).await, at(21))
        .await
        .expect("receive 3 of 4");
    let sql = format!(
        "SELECT count(*) FROM cash_exceptions WHERE branch_id = '{}' AND data->>'transferId' = '{}' \
         AND (data->>'difference')::numeric = -50",
        s.b1.as_str(),
        t.id
    );
    assert_eq!(db.scalar(&sql).await, 1);
}

// ── #621 rework: rulings Q1, Q2 and the tightened CHECK (added by claude/dell-claude-6) ──────────────

/// A within-limit move (`notes` × USD 50 ≤ the USD 300 limit) on `s.b1`, created and submitted by FINANCE.
async fn submitted_within_limit(db: &Db, s: &Shop, notes: i32) -> CashTransfer {
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ($1, $2::uuid, jsonb_build_object('USD', '300.00'::text), $3::uuid)",
    )
    .bind(unique("v"))
    .bind(ORG)
    .bind(&s.exec.user_id)
    .execute(&db.pool)
    .await
    .expect("a transfer policy version");
    let flow = db.flow();
    let cmd = NewTransfer {
        kind: TransferKind::ExternalIn,
        from_branch: s.b1.clone(),
        to_branch: None,
        from_shift: None,
        to_shift: None,
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: db.usd("50").await,
            qty: notes,
        }],
        note: None,
        idempotency_key: unique("q2"),
    };
    let t = flow
        .create(&s.finance, cmd, at(10))
        .await
        .expect("FINANCE creates the move");
    flow.submit(&s.finance, &t.id, at(11))
        .await
        .expect("submit")
}

#[tokio::test]
async fn q2_within_the_limit_finance_does_not_approve_even_holding_approvals_decide() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let t = submitted_within_limit(&db, &s, 2).await; // USD 100 ≤ 300
    let finance_b1 = db.person("Fai", &[(Role::Finance, Some(&s.b1))]).await;
    let finance_org = db.person("Org", &[(Role::Finance, None)]).await;
    for who in [&finance_b1, &finance_org] {
        assert!(
            db.flow()
                .decide(who, &t.id, Outcome::Approved, "ok", at(20))
                .await
                .is_err(),
            "FINANCE never decides within the limit"
        );
    }
    assert_eq!(
        db.scalar(&format!(
            "SELECT count(*) FROM cash_transfers WHERE id = '{}'::uuid AND status = 'DRAFT'",
            t.id
        ))
        .await,
        1
    );
    let t = db
        .flow()
        .decide(&s.manager, &t.id, Outcome::Approved, "ok", at(21))
        .await
        .expect("the source branch's BRANCH_MANAGER approves within the limit");
    assert_eq!(t.status, TransferStatus::Approved);
}

#[tokio::test]
async fn q2_within_the_limit_only_the_source_branch_manager_approves_not_the_executive() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let t = submitted_within_limit(&db, &s, 2).await;
    assert!(
        db.flow()
            .decide(&s.exec, &t.id, Outcome::Approved, "ok", at(20))
            .await
            .is_err(),
        "within the limit the approver is the source BRANCH_MANAGER (A5)"
    );
    let b2 = db.branch("b2", 0).await;
    let manager_b2 = db.person("Nida", &[(Role::BranchManager, Some(&b2))]).await;
    assert!(
        db.flow()
            .decide(&manager_b2, &t.id, Outcome::Approved, "ok", at(21))
            .await
            .is_err(),
        "a BRANCH_MANAGER of another branch"
    );
}

#[tokio::test]
async fn an_approver_without_approvals_decide_is_refused_and_the_request_stays_pending() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let t = submitted_within_limit(&db, &s, 2).await;
    let admin = db.person("Adm", &[(Role::Admin, None)]).await;
    for who in [&s.teller, &admin] {
        assert!(
            db.flow()
                .decide(who, &t.id, Outcome::Approved, "ok", at(20))
                .await
                .is_err(),
            "{}",
            who.user_id
        );
    }
    assert_eq!(
        db.scalar(&format!(
            "SELECT count(*) FROM approval_requests WHERE status = 'PENDING' AND id = \
             (SELECT approval_request_id FROM cash_transfers WHERE id = '{}'::uuid)",
            t.id
        ))
        .await,
        1,
        "the request is still pending"
    );
}

#[tokio::test]
async fn q1_a_branch_manager_who_is_also_a_teller_of_the_branch_does_not_receive_into_another_tellers_drawer()
 {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let shift = db.drawer(&s.b1, &s.teller, &s.manager, 1).await;
    let t = dispatched(&db, &s, TransferKind::SafeToDrawer, None, Some(&shift), 2)
        .await
        .expect("dispatch");
    let both = db
        .person(
            "Both",
            &[
                (Role::Teller, Some(&s.b1)),
                (Role::BranchManager, Some(&s.b1)),
            ],
        )
        .await;
    assert!(
        db.flow()
            .receive(&both, &t.id, counted(&db, 2).await, at(20))
            .await
            .is_err(),
        "the person rule is the to_shift's teller, by user id"
    );
    db.flow()
        .receive(&s.teller, &t.id, counted(&db, 2).await, at(21))
        .await
        .expect("the to_shift's teller receives");
}

#[tokio::test]
async fn the_check_requires_the_drawer_end_of_each_safe_move() {
    let db = Db::fresh().await;
    let s = shop(&db).await;
    let shift = db.drawer(&s.b1, &s.teller, &s.manager, 1).await;
    let insert = |kind: &'static str, from_shift: Option<String>, to_shift: Option<String>| {
        let pool = db.pool.clone();
        let b1 = s.b1.as_str().to_owned();
        let by = s.finance.user_id.clone();
        async move {
            sqlx::query(
                "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, from_shift_id, to_shift_id, status, created_by, idempotency_key) \
                 VALUES ($1::uuid, $2, $3, $4, $5, 'DRAFT', $6::uuid, $7)",
            )
            .bind(ORG)
            .bind(kind)
            .bind(b1)
            .bind(from_shift)
            .bind(to_shift)
            .bind(by)
            .bind(unique("k"))
            .execute(&pool)
            .await
        }
    };
    for (kind, from_shift, to_shift) in [
        ("DRAWER_TO_SAFE", None, None),
        ("DRAWER_TO_SAFE", None, Some(shift.clone())),
        ("SAFE_TO_DRAWER", None, None),
        ("SAFE_TO_DRAWER", Some(shift.clone()), None),
    ] {
        let err = insert(kind, from_shift.clone(), to_shift.clone())
            .await
            .expect_err(kind);
        let code = err
            .as_database_error()
            .and_then(|e| e.code())
            .map(|c| c.into_owned());
        assert_eq!(
            code.as_deref(),
            Some("23514"),
            "{kind} {from_shift:?}/{to_shift:?}: the CHECK refuses it: {err}"
        );
    }
    insert("DRAWER_TO_SAFE", Some(shift.clone()), None)
        .await
        .expect("DRAWER_TO_SAFE with its source drawer");
    insert("SAFE_TO_DRAWER", None, Some(shift.clone()))
        .await
        .expect("SAFE_TO_DRAWER with its destination drawer");
}

// ── #621 second rework: one transfer dispatched twice at once moves the cash once (added by claude/dell-claude-6) ──

#[tokio::test]
async fn two_concurrent_dispatches_of_one_approved_transfer_move_the_safe_once() {
    let db = Db::fresh().await;
    let s = shop(&db).await; // USD 500.00 in B1's safe
    for round in 0..10 {
        let flow = db.flow();
        let cmd = NewTransfer {
            kind: TransferKind::ExternalOut,
            from_branch: s.b1.clone(),
            to_branch: None,
            from_shift: None,
            to_shift: None,
            lines: vec![NewTransferLine {
                currency: usd(),
                denomination_id: db.usd("50").await,
                qty: 1,
            }],
            note: None,
            idempotency_key: unique("race"),
        };
        let t = flow
            .create(&s.finance, cmd, at(10))
            .await
            .expect("FINANCE creates the move");
        flow.submit(&s.finance, &t.id, at(11))
            .await
            .expect("submit");
        flow.decide(&s.exec, &t.id, Outcome::Approved, "ok", at(12))
            .await
            .expect("EXECUTIVE approves");

        let before = db.safe_usd(&s.b1).await;
        let (a, b) = (db.flow(), db.flow());
        let (first, second) = tokio::join!(
            a.dispatch(&s.finance, &t.id, at(13)),
            b.dispatch(&s.finance, &t.id, at(13)),
        );
        let oks = [&first, &second].iter().filter(|r| r.is_ok()).count();
        assert_eq!(
            oks, 1,
            "round {round}: exactly one dispatch wins: {first:?} / {second:?}"
        );
        let loser = if first.is_ok() { second } else { first };
        refused_with(
            loser,
            TransferError::IllegalTransition {
                from: TransferStatus::Dispatched,
                step: xc_cash::domain::transfer::cash_transfer::TransferStep::Dispatch,
            },
        );
        assert_eq!(
            db.safe_usd(&s.b1).await,
            before - 5_000,
            "round {round}: USD 50 left the safe once"
        );
    }
}
