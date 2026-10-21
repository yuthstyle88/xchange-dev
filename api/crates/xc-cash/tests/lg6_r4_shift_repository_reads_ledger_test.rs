//! Up-front tests for LG-6.r4 (#1043): `xc-shift`'s `PgShiftRepository` reads a shift's balances
//! through the `ShiftBalances` port (design `docs/design/blueprint/ledger-inventory.md` §4.2.1, PR 1037)
//! and `ShiftService` no longer writes its own copy in `shift_balances`.
//! Written before the implementation by claude/d7aa64 and re-cut by claude/dell-claude-3 on
//! `stack/626-base` (after CA-10 #626 removed `open_in`/`move_stock_in`/`close_in`/`apply_delta_in`/
//! `active`/`active_in`; the shift is now opened, moved and closed only through the count flows and
//! `apply_delta_for_teller_in`). Ticket author's test commit. Do not edit, delete or weaken.
//!
//! They live in `xc-cash` because only `xc-cash` can name both sides: `xc-shift`'s
//! `PgShiftRepository`/`ShiftService` and `xc-cash`'s `PgLedgerShiftBalances`/`LedgerCashPostings`
//! (`xc-cash` depends on `xc-shift`, §4.2.1).
//!
//! The discriminating move in each test: a posting reaches the ledger **without** going through
//! `ShiftService` (so the old `shift_balances` copy does not see it), or a count is verified without
//! a `close_shift` write. A repository that still reads the copy answers the copy's figure; the one
//! source answers the ledger's.
//!
//! The repository is built with `PgShiftRepository::with_balances(pool, balances)` (ruling B2 on #1043,
//! design §4.2.1). `PgShiftRepository::new(pool)` stays and has no port, so its balance reads answer
//! an error, never the stale copy.
//!
//! `PgLedgerShiftBalances` reads on its own pool, so every posting is committed before it is read.
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_cash::application::count::count_service::post_opening_in;
use xc_cash::application::ledger_approval::negative_stock::NegativeStockSource;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{
    ApprovalRequestId, CashPlace, CashPostings, PostingRequest, TransactionLineId,
};
use xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, MoneyDelta};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::entities::shift::{Shift, ShiftStatus};
use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;
use xc_shift::domain::shift::repositories::shift_repository::ShiftRepository;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

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

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a valid decimal")
}

fn money(s: &str) -> Money {
    Money::parse(s).expect("a valid money")
}

fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).unwrap_or_else(|_| panic!("a currency code: {s}"))
}

fn thb() -> CurrencyCode {
    code("THB")
}

fn usd() -> CurrencyCode {
    code("USD")
}

fn now() -> DateTime<Utc> {
    // Whole milliseconds, as `ShiftService` truncates them.
    DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).expect("a valid instant")
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

struct Env {
    pool: PgPool,
    branch: BranchId,
    ledger: LedgerCashPostings,
    service: ShiftService<PgShiftRepository>,
}

/// A fresh schema with every migration applied, one branch, and a
/// `ShiftService` whose repository reads balances through `PgLedgerShiftBalances` (§3.1 of #1043:
/// `PgShiftRepository::with_balances(pool, balances)`, the lane controller's ruling B2: `new(pool)` stays).
async fn env() -> Env {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
             Point it at an existing database whose name ends in `_test`."
        )
    });
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(
        options.get_database().unwrap_or("").ends_with("_test"),
        "TEST_DATABASE_URL must name a database ending in `_test`"
    );
    let schema = unique("lg6r4");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .expect("connect to the test database");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create the test schema");
    admin.close().await;

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the test schema");
    Migrator::new(migrations_dir().as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies on a fresh database");

    let branch = unique("B");
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(&branch)
        .execute(&pool)
        .await
        .expect("insert a branch");

    let balances: Arc<dyn ShiftBalances> = Arc::new(PgLedgerShiftBalances::new(pool.clone()));
    let repo = PgShiftRepository::with_balances(pool.clone(), balances);
    let service = ShiftService::new(Arc::new(repo), BranchTx::new(Transactor::new(pool.clone())));

    Env {
        pool,
        branch: BranchId::new(branch),
        ledger: LedgerCashPostings::new(),
        service,
    }
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'Test Teller', 'ACTIVE', 'test')",
    )
    .bind(id)
    .bind(ORG)
    .execute(pool)
    .await
    .expect("insert user");
    id
}

fn src(source_type: SourceType, key: &str) -> SourceRef {
    SourceRef {
        source_type,
        source_id: unique("src"),
        source_key: key.to_string(),
    }
}

fn req(
    from: CashPlace,
    to: CashPlace,
    currency: CurrencyCode,
    quantity: &str,
    source: &SourceRef,
    line: Option<Uuid>,
) -> PostingRequest {
    PostingRequest {
        from,
        to,
        currency,
        quantity: ForeignAmount::from_decimal(d(quantity)),
        source: source.clone(),
        line,
        reverses: None,
    }
}

fn drawer(shift: &str) -> CashPlace {
    CashPlace::Drawer(shift.to_string())
}

impl Env {
    fn safe(&self) -> CashPlace {
        CashPlace::Safe(self.branch.clone())
    }

    /// One committed posting through the real port, outside `ShiftService`.
    async fn post(&self, reqs: &[PostingRequest], what: &str) {
        let mut tx = self.pool.begin().await.expect("begin");
        self.ledger
            .post_in(&mut tx, reqs, None)
            .await
            .unwrap_or_else(|e| panic!("{what}: the port refused: {e:?}"));
        tx.commit().await.expect("commit");
    }

    /// The safe holds `amounts` (`BANK → SAFE`), so a drawer can open from it.
    async fn fund_safe(&self, amounts: &[(CurrencyCode, &str)]) {
        for (c, a) in amounts {
            let s = src(SourceType::Opening, "fund");
            self.post(
                &[req(CashPlace::Bank, self.safe(), *c, a, &s, None)],
                "fund the safe",
            )
            .await;
        }
    }

    /// A verified OPEN count opens a shift for a new teller, as the count flow does it
    /// (`count_service.rs` `open_on_verified`, then `post_opening_in` from the safe), in one committed
    /// transaction. After M3 one teller holds one OPEN shift, so every open gets its own teller.
    async fn open(&self, opening: &[(CurrencyCode, &str)]) -> Opened {
        let teller = seed_user(&self.pool).await;
        let verifier = seed_user(&self.pool).await;
        let count: Uuid = sqlx::query_scalar(
            "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, \
             verified_by, verified_at, idempotency_key) \
             VALUES ($1::uuid, $2, NULL, 'OPEN', $3, now(), $4, now(), $5) RETURNING id",
        )
        .bind(ORG)
        .bind(self.branch.as_str())
        .bind(teller)
        .bind(verifier)
        .bind(unique("open-count"))
        .fetch_one(&self.pool)
        .await
        .expect("insert the verified OPEN count");
        let lines: Vec<(String, Decimal)> = opening
            .iter()
            .map(|(c, a)| (c.as_str().to_string(), d(a)))
            .collect();
        let amounts: Vec<(CurrencyCode, Decimal)> =
            opening.iter().map(|(c, a)| (*c, d(a))).collect();
        let mut tx = self.pool.begin().await.expect("begin");
        let shift = self
            .service
            .open_on_verified(
                &mut tx,
                &self.branch,
                &teller.to_string(),
                &count.to_string(),
                &lines,
                now(),
            )
            .await
            .expect("open on the verified count");
        post_opening_in(
            &self.ledger,
            &mut tx,
            self.safe(),
            &shift.id,
            &amounts,
            &count.to_string(),
        )
        .await
        .expect("post the opening");
        tx.commit().await.expect("commit");
        Opened {
            shift,
            teller: teller.to_string(),
            verifier: verifier.to_string(),
        }
    }

    /// A drawer ↔ safe `TRANSFER` posted straight to the ledger (not through `ShiftService`).
    async fn transfer(&self, shift: &str, currency: CurrencyCode, qty: &str, into_drawer: bool) {
        let s = src(SourceType::Transfer, "move");
        let r = if into_drawer {
            req(self.safe(), drawer(shift), currency, qty, &s, None)
        } else {
            req(drawer(shift), self.safe(), currency, qty, &s, None)
        };
        self.post(&[r], "a drawer/safe transfer").await;
    }

    /// A cash SELL of `qty` USD for `thb` THB, posted the way `CompleteExchange` posts it (§4.3):
    /// the drawer gives the foreign currency and takes the THB.
    async fn exchange_sell_usd(&self, shift: &str, qty: &str, thb_amount: &str) {
        let s = src(SourceType::Exchange, "complete");
        self.post(
            &[
                req(
                    drawer(shift),
                    CashPlace::Customer,
                    usd(),
                    qty,
                    &s,
                    Some(Uuid::new_v4()),
                ),
                req(
                    CashPlace::Customer,
                    drawer(shift),
                    thb(),
                    thb_amount,
                    &s,
                    None,
                ),
            ],
            "an exchange",
        )
        .await;
    }

    /// `apply_delta_for_teller_in` (the save path, `save_transaction.rs`) in a transaction of its own,
    /// rolled back after: only the answer matters.
    async fn apply_delta_for_teller(
        &self,
        teller: &str,
        delta: &StockDelta,
    ) -> Result<String, AppError> {
        let mut tx = self.pool.begin().await.expect("begin");
        let out = self
            .service
            .apply_delta_for_teller_in(&mut tx, &self.branch, teller, delta)
            .await;
        tx.rollback().await.expect("rollback");
        out
    }

    async fn finish(self) {
        self.pool.close().await;
    }
}

/// A shift opened by [`Env::open`], with the user who holds it and the user who verified its count.
struct Opened {
    shift: Shift,
    teller: String,
    verifier: String,
}

fn usd_out(amount: &str) -> StockDelta {
    let minor = (d(amount) * Decimal::from(100)).round().mantissa();
    StockDelta::of(&[(usd(), MoneyDelta::from_minor(-minor))])
}

fn insufficient(c: CurrencyCode) -> String {
    format!("{:?}", AppError::from(ShiftError::Insufficient(c)))
}

fn balances(pairs: &[(CurrencyCode, &str)]) -> Balances {
    let pairs: Vec<(CurrencyCode, Money)> = pairs.iter().map(|(c, a)| (*c, money(a))).collect();
    Balances::of(&pairs)
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 row 1: a sale that would take the drawer below zero → `Insufficient`, read from the ledger
// ─────────────────────────────────────────────────────────────────────────────

/// #1043 §9 row 1 on the one save path left after CA-10 (`apply_delta_for_teller_in`,
/// `save_transaction.rs`). The drawer opens with USD 100; USD 60 then leaves it for the safe in the
/// ledger only. A sale of USD 50 needs more than the ledger's USD 40, so it is refused with
/// `Insufficient(USD)`. Reading the `shift_balances` copy (USD 100) accepts it.
#[tokio::test]
async fn a_sale_beyond_the_ledgers_drawer_is_refused_even_when_the_old_copy_would_allow_it() {
    let env = env().await;
    env.fund_safe(&[(usd(), "1000")]).await;
    let o = env.open(&[(usd(), "100")]).await;
    env.transfer(&o.shift.id, usd(), "60", false).await;

    match env.apply_delta_for_teller(&o.teller, &usd_out("50")).await {
        Err(e) => assert_eq!(
            format!("{e:?}"),
            insufficient(usd()),
            "the drawer holds USD 40 in the ledger"
        ),
        Ok(id) => panic!(
            "apply_delta_for_teller_in accepted a USD 50 sale from shift {id}, whose drawer holds \
             USD 40 in the ledger: it read the shift_balances copy (USD 100), not ShiftBalances"
        ),
    }

    env.finish().await;
}

/// #1043 §9 row 1, the other direction: USD 200 comes into the drawer in the ledger only, so the
/// ledger holds USD 300 while the copy says USD 100. A sale of USD 250 is accepted on that shift.
#[tokio::test]
async fn a_sale_within_the_ledgers_drawer_is_accepted_even_when_the_old_copy_would_refuse_it() {
    let env = env().await;
    env.fund_safe(&[(usd(), "1000")]).await;
    let o = env.open(&[(usd(), "100")]).await;
    env.transfer(&o.shift.id, usd(), "200", true).await;

    assert_eq!(
        env.apply_delta_for_teller(&o.teller, &usd_out("250"))
            .await
            .map_err(|e| format!("{e:?}")),
        Ok(o.shift.id.clone()),
        "the drawer holds USD 300 in the ledger; a USD 250 sale must be accepted on this shift"
    );

    env.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 row 3: a read right after an exchange posts → `expected` includes it (one source)
// ─────────────────────────────────────────────────────────────────────────────

/// #1043 §9 row 3 and §3.1. The drawer opens with THB 5000 and USD 100; a cash sale of USD 30 for
/// THB 1050 is then posted as `CompleteExchange` posts it, outside `ShiftService`. Every read that
/// builds a `Shift` (`shift`, `shifts`, `open_of_teller`, `open_of_teller_in`,
/// `find_shift_by_open_request_id`) answers the ledger: `expected` THB 6050 / USD 70, `opening`
/// THB 5000 / USD 100.
#[tokio::test]
async fn every_read_of_an_open_shift_answers_the_ledgers_expected_right_after_an_exchange_posts() {
    let env = env().await;
    env.fund_safe(&[(thb(), "100000"), (usd(), "1000")]).await;
    let o = env.open(&[(thb(), "5000"), (usd(), "100")]).await;
    env.exchange_sell_usd(&o.shift.id, "30", "1050").await;

    let want_expected = balances(&[(thb(), "6050"), (usd(), "70")]);
    let want_opening = balances(&[(thb(), "5000"), (usd(), "100")]);
    let repo = env.service.repo();

    let one = repo
        .shift(&env.branch, &o.shift.id)
        .await
        .expect("shift")
        .expect("the shift exists");
    assert_eq!(one.expected, want_expected, "shift().expected");
    assert_eq!(one.opening, want_opening, "shift().opening");

    let listed = env.service.shifts(&env.branch).await.expect("shifts");
    let listed = listed
        .iter()
        .find(|s| s.id == o.shift.id)
        .expect("the shift is listed");
    assert_eq!(listed.expected, want_expected, "shifts()[..].expected");
    assert_eq!(listed.opening, want_opening, "shifts()[..].opening");

    let of_teller = env
        .service
        .open_of_teller(&o.teller)
        .await
        .expect("open_of_teller")
        .expect("the teller's shift is open");
    assert_eq!(of_teller.id, o.shift.id);
    assert_eq!(
        of_teller.expected, want_expected,
        "open_of_teller().expected"
    );

    let mut tx = env.pool.begin().await.expect("begin");
    let in_tx = env
        .service
        .open_of_teller_in(&mut tx, &o.teller)
        .await
        .expect("open_of_teller_in")
        .expect("the teller's shift is open");
    assert_eq!(
        in_tx.expected, want_expected,
        "open_of_teller_in().expected"
    );
    let by_request = repo
        .find_shift_by_open_request_id(&mut tx, &env.branch, &o.shift.open_request_id)
        .await
        .expect("find_shift_by_open_request_id")
        .expect("found by its open request");
    assert_eq!(
        by_request.expected, want_expected,
        "find_shift_by_open_request_id().expected"
    );
    tx.rollback().await.expect("rollback");

    env.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §3.1: a closed shift's counted and difference come from its verified count
// ─────────────────────────────────────────────────────────────────────────────

/// #1043 §3.1: `Shift.counted` / `difference` are filled from `ShiftBalance`, i.e. the shift's
/// verified CLOSE count (`cash_count_totals`, §4.2.1). The shift here was closed by its count only;
/// nothing wrote `shift_balances`. A repository reading the copy answers an empty count.
#[tokio::test]
async fn a_closed_shifts_counted_and_difference_come_from_its_verified_close_count() {
    let env = env().await;
    env.fund_safe(&[(usd(), "1000")]).await;
    let o = env.open(&[(usd(), "100")]).await;
    sqlx::query(
        "UPDATE shifts SET status = 'CLOSED', closed_at = now(), closed_by = 'test', close_note = 'short' \
         WHERE id = $1",
    )
    .bind(&o.shift.id)
    .execute(&env.pool)
    .await
    .expect("close the shift row");
    let count: Uuid = sqlx::query_scalar(
        "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, \
         verified_by, verified_at, idempotency_key) \
         VALUES ($1::uuid, $2, $3, 'CLOSE', $4::uuid, now(), $5::uuid, now(), $6) RETURNING id",
    )
    .bind(ORG)
    .bind(env.branch.as_str())
    .bind(&o.shift.id)
    .bind(&o.teller)
    .bind(&o.verifier)
    .bind(unique("count"))
    .fetch_one(&env.pool)
    .await
    .expect("insert the verified CLOSE count");
    sqlx::query(
        "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, difference) \
         VALUES ($1, $2::uuid, 'USD', 95, 100, -5)",
    )
    .bind(count)
    .bind(ORG)
    .execute(&env.pool)
    .await
    .expect("insert the count's USD total");

    let closed = env
        .service
        .repo()
        .shift(&env.branch, &o.shift.id)
        .await
        .expect("shift")
        .expect("the shift exists");
    assert_eq!(closed.status, ShiftStatus::Closed);
    assert_eq!(
        closed.counted,
        Some(balances(&[(usd(), "95")])),
        "counted comes from the verified CLOSE count"
    );
    assert_eq!(
        closed.difference,
        Some(StockDelta::of(&[(usd(), MoneyDelta::from_minor(-500))])),
        "difference comes from the verified CLOSE count"
    );

    env.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §3.2 / §4 done criterion 2: no `shift_balances` write is left
// ─────────────────────────────────────────────────────────────────────────────

/// #1043 §3.2 on every writer left after CA-10: `open_on_verified` (the OPEN count and the
/// hand-over, each followed by `post_opening_in`), `apply_delta_for_teller_in` (the save path) and
/// `close_on_verified` (the CLOSE count, whose totals are `cash_count_totals`). `shift_balances` is
/// dropped by #1047, the shift reads its balances from the ledger, and the close still writes
/// the `shifts` row.
#[tokio::test]
async fn the_count_flows_and_the_save_path_write_no_shift_balances_row() {
    let env = env().await;
    env.fund_safe(&[(usd(), "1000")]).await;
    let o = env.open(&[(usd(), "100")]).await;

    let mut tx = env.pool.begin().await.expect("begin");
    env.service
        .apply_delta_for_teller_in(&mut tx, &env.branch, &o.teller, &usd_out("30"))
        .await
        .expect("apply a USD 30 sale");
    tx.commit().await.expect("commit");
    env.exchange_sell_usd(&o.shift.id, "30", "1050").await;

    env.transfer(&o.shift.id, usd(), "10", false).await;
    let read = env
        .service
        .repo()
        .shift(&env.branch, &o.shift.id)
        .await
        .expect("shift")
        .expect("the shift exists");
    assert_eq!(
        read.expected,
        balances(&[(thb(), "1050"), (usd(), "60")]),
        "a count-opened shift reads its expected from the ledger"
    );

    let mut tx = env.pool.begin().await.expect("begin");
    env.service
        .close_on_verified(
            &mut tx,
            &o.shift.id,
            &o.verifier,
            &[
                ("THB".to_string(), d("1050"), d("0")),
                ("USD".to_string(), d("59"), d("-1")),
            ],
            "one dollar short",
            now(),
        )
        .await
        .expect("close on the verified count");
    tx.commit().await.expect("commit");

    let status: String = sqlx::query_scalar("SELECT status FROM shifts WHERE id = $1")
        .bind(&o.shift.id)
        .fetch_one(&env.pool)
        .await
        .expect("read the shift row");
    assert_eq!(
        status, "CLOSED",
        "close_on_verified still writes the shifts row"
    );

    env.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Design §4.2.1: a repository built by `new` has no port
// ─────────────────────────────────────────────────────────────────────────────

/// Design §4.2.1 ("How production builds a `PgShiftRepository`"): `PgShiftRepository::new(pool)` has
/// no `ShiftBalances` port, so a read that builds a `Shift` answers an error, never the stale
/// `shift_balances` copy. The shift's rows exist; only the port is missing.
#[tokio::test]
async fn a_repository_built_without_the_port_refuses_to_build_a_shift_never_reads_the_copy() {
    let env = env().await;
    env.fund_safe(&[(usd(), "1000")]).await;
    let o = env.open(&[(usd(), "100")]).await;
    let portless = PgShiftRepository::new(env.pool.clone());

    match portless.shift(&env.branch, &o.shift.id).await {
        Err(_) => {}
        Ok(s) => panic!(
            "PgShiftRepository::new built shift {:?} with expected {:?}: a repository with no \
             ShiftBalances port must answer an error, never the shift_balances copy",
            s.as_ref().map(|s| &s.id),
            s.as_ref().map(|s| &s.expected)
        ),
    }

    env.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Ruling on #1043 (lane controller, 2026-10-15): a negative ledger expected is an error, never 0
// ─────────────────────────────────────────────────────────────────────────────

/// A transaction line for the short sale (`xc-exchange` is not a dependency of `xc-cash`, so SQL;
/// the same seed as `shift_balances_test.rs`).
async fn seed_sell_line(
    pool: &PgPool,
    branch: &BranchId,
    shift: &str,
    qty: &str,
    thb_amount: &str,
) -> (TransactionLineId, String) {
    let cust_id = unique("CU");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $3, 'Somchai', 'LIVE', now())",
    )
    .bind(&cust_id)
    .bind(ORG)
    .bind(unique("CUSTNO"))
    .execute(pool)
    .await
    .expect("seed customer");
    let tx_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
         customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
         recorded_at, recorded_by, status, completed_at) \
         VALUES ($1, $2::uuid, $3, $4, 'LIVE', $5, $6, 'hash', $7, 'Somchai', 'THA', 'system', 'WALK-IN', \
         '', current_date, now(), now(), 'system', 'COMPLETED', now())",
    )
    .bind(tx_id)
    .bind(ORG)
    .bind(unique("DOC"))
    .bind(branch.as_str())
    .bind(shift)
    .bind(unique("sub"))
    .bind(&cust_id)
    .execute(pool)
    .await
    .expect("insert transaction");
    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
         quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, cost_basis_thb, \
         realized_margin_thb, line_no, created_at, updated_at) \
         VALUES ($1, $2::uuid, $3, 'SELL', 'USD', $4, 1, 1.0, $5, 0, $5, NULL, NULL, 1, now(), now())",
    )
    .bind(line_id)
    .bind(ORG)
    .bind(tx_id)
    .bind(d(qty))
    .bind(d(thb_amount))
    .execute(pool)
    .await
    .expect("insert transaction line");
    (line_id, tx_id.to_string())
}

/// An APPROVED `NEGATIVE_STOCK` request for `object_id`, at the hash the port computes now.
async fn approved_negative_stock(
    pool: &PgPool,
    ledger: &LedgerCashPostings,
    branch: &BranchId,
    object_id: &str,
) -> ApprovalRequestId {
    let repo = PgApprovalRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    let hash = NegativeStockSource::version_hash(ledger, &mut tx, object_id)
        .await
        .expect("the source's version hash");
    let request = repo
        .insert_request(
            &mut tx,
            ORG,
            "NEGATIVE_STOCK",
            Some(branch.as_str().to_string()),
            object_id,
            &hash,
            "v1",
            serde_json::json!({}),
            "teller",
            &unique("key"),
        )
        .await
        .expect("insert approval request");
    repo.update_status(&mut tx, &request.id, "APPROVED")
        .await
        .expect("approve the request");
    tx.commit().await.expect("commit");
    ApprovalRequestId::new(request.id.as_str())
}

/// The error must name the currency whose drawer is negative.
fn assert_names_usd<T: std::fmt::Debug>(what: &str, answer: Result<T, AppError>) {
    match answer {
        Ok(v) => panic!(
            "{what} answered Ok({v:?}) for a shift whose ledger drawer holds USD -6.00: a negative \
             ledger expected must be an error naming the currency, never clamped to 0 or dropped"
        ),
        Err(e) => assert!(
            e.to_string().contains("USD"),
            "{what}: the error for a negative USD drawer must name USD; got `{e}` ({e:?})"
        ),
    }
}

/// Ruling on #1043 (lane controller, 2026-10-15): when the ledger's `ShiftBalance.expected` for a
/// currency is negative, the read that builds `Shift.expected` (a non-negative `Balances`) returns an
/// error naming that currency. It never clamps to 0. Every expected figure in `v2_shift_cases.json` is
/// non-negative (1,510 values), so no recorded case reaches this.
///
/// The drawer opens with USD 4, then an approved `NEGATIVE_STOCK` cash sale of USD 10 for THB 330 is
/// posted straight through `LedgerCashPostings`: USD −6.00 in the ledger. `open_of_teller()` and
/// `shift()` must both refuse to build the shift, with an error whose text names USD.
#[tokio::test]
async fn a_negative_ledger_expected_is_an_error_naming_the_currency_never_zero() {
    let env = env().await;
    env.fund_safe(&[(usd(), "4")]).await;
    let o = env.open(&[(usd(), "4")]).await;

    let (line, sale) = seed_sell_line(&env.pool, &env.branch, &o.shift.id, "10", "330.00").await;
    let approval = approved_negative_stock(&env.pool, &env.ledger, &env.branch, &sale).await;
    let s = SourceRef {
        source_type: SourceType::Exchange,
        source_id: sale.clone(),
        source_key: "complete".to_string(),
    };
    let mut tx = env.pool.begin().await.expect("begin");
    env.ledger
        .post_in(
            &mut tx,
            &[
                req(
                    drawer(&o.shift.id),
                    CashPlace::Customer,
                    usd(),
                    "10",
                    &s,
                    Some(line),
                ),
                req(
                    CashPlace::Customer,
                    drawer(&o.shift.id),
                    thb(),
                    "330.00",
                    &s,
                    None,
                ),
            ],
            Some(approval),
        )
        .await
        .expect("post the approved short sale");
    tx.commit().await.expect("commit");

    assert_names_usd(
        "open_of_teller()",
        env.service.open_of_teller(&o.teller).await,
    );
    assert_names_usd(
        "shift()",
        env.service.repo().shift(&env.branch, &o.shift.id).await,
    );

    env.finish().await;
}
