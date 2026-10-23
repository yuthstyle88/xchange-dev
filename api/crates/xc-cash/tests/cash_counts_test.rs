//! CA-3 (#619 Part 2): counts and shifts per teller (design `docs/design/blueprint/cash-shift-transfer.md` §3.1,
//! §4.2, §4.3, §4.6, M2 in §8; the cases are §9.1's CA-3 row, plus `TooManyDecimals` moved here from CA-1).
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.
//!
//! [required] by these tests (ticket §7):
//! - `xc_cash::application::count::{CashCountService, SubmitCount}`:
//!   - `CashCountService::pg(pool: PgPool) -> Self` wires the Pg repository, `LedgerCashPostings` (LG-4 #929) and the shift
//!     service;
//!   - `submit(&self, actor: &Actor, cmd: SubmitCount, now: DateTime<Utc>) -> AppResult<CashCount>`;
//!   - `verify(&self, actor: &Actor, count_id: &str, now: DateTime<Utc>) -> AppResult<CashCount>`.
//!   - `SubmitCount { branch: BranchId, shift_id: Option<String>, kind: CountKind, lines: Vec<CountLine>, note:
//!     Option<String>, idempotency_key: String }`.
//! - `xc_cash::domain::count::{CashCount, CountKind, CountLine}`:
//!   - `CountKind::{Open, HandoverOut, Close, SafeOpen, Safe}`;
//!   - `CountLine::new(currency: &str, denomination_id: i64, qty: i32)`;
//!   - `CashCount` has the public fields `id: String`, `shift_id: Option<String>`, `counted_by: String` and
//!     `verified_by: Option<String>` (users ids).
//! - `xc_cash::domain::error::CashError` also has `NotNewestCount`, `BalancesChanged`, `TooManyDecimals`,
//!   `ShiftAlreadyOpen` and `Refused(Refusal)` (`xc_auth::domain::access::Refusal`). Every `CashError` reaches the
//!   caller as `AppError::Domain(error.to_string())`, the convention of `ShiftError` (`shift_error.rs`).
//! - `ShiftService::apply_delta_for_teller_in(&self, tx, branch: &BranchId, teller: &str, delta: &StockDelta) ->
//!   AppResult<String>` (the id of the teller's OPEN shift it moved). It is added beside `apply_delta_in`, which
//!   `save_transaction.rs` keeps calling until CA-7 (that file is CA-7's).
//! - Migration `20261031080000_shift_per_teller.sql` (M2, after every migration on main; option A of the lane controller's 2026-10-10 ruling; design
//!   §4.1, §8 M2): `shifts.opened_by_count uuid NULL REFERENCES cash_counts(id)`, set to the verified count that
//!   opened the shift; `shifts_one_open_per_teller` over count-opened OPEN shifts; `shifts_one_open_per_branch_v2`
//!   over v2-opened OPEN shifts (`opened_by_count IS NULL`); the per-branch index dropped. **M2's four tests here
//!   retired with CA-10 (#626):** its M3 drops the marker and the v2 index; the end state is pinned by
//!   `xc-shift/tests/ca10_m3_migration_test.rs`.
//! - A non-zero difference on a verified count writes one `cash_exceptions` row of the count's branch whose `data`
//!   has `countId` (the count's id), `currency` and `difference` (a decimal string, counted − expected).
//!
//! What the tests read from the database: `shifts.teller_id` and `status`, `shift_balances.opening` / `expected` /
//! `counted`, `cash_count_totals`, `cash_counts`, `cash_exceptions`, and the indexes on `shifts`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Refusal, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CashCount, CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

// ── the database ────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
    migrator: Migrator,
}

impl Db {
    async fn new() -> Db {
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
        let schema = unique("ca3");
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
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        Db { pool, migrator }
    }

    async fn fresh() -> Db {
        let db = Db::new().await;
        db.migrator
            .run(&db.pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        db
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert a branch of the organisation");
        lg4_fund_safe(&self.pool, &id, "THB", "1000000.00").await;
        BranchId::new(id)
    }

    /// A user of the organisation holding `roles` organisation-wide, and the `Actor` for them.
    async fn person(&self, name: &str, roles: &[Role]) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        for role in roles {
            sqlx::query(
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
                 VALUES ($1::uuid, $2, $3::uuid, NULL)",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
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
                .map(|r| ScopedRole::new(*r, None, from, None))
                .collect(),
            None,
        )
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} {face} face value (A7): {e}"))
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn open_shifts_of(&self, teller: &Actor) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT id, branch_id FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN' ORDER BY seq",
        )
        .bind(&teller.user_id)
        .fetch_all(&self.pool)
        .await
        .expect("read the teller's open shifts")
    }

    async fn shift(&self, id: &str) -> (String, String) {
        sqlx::query_as("SELECT teller_id::text, status FROM shifts WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read the shift")
    }

    /// `(opening, expected, counted)` of one currency of a shift.
    async fn balance(&self, shift: &str, currency: &str) -> (Decimal, Decimal, Option<Decimal>) {
        use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;
        let rows = xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances::new(
            self.pool.clone(),
        )
        .shift_balances(&shift.to_owned())
        .await
        .unwrap_or_else(|e| panic!("the {currency} balance of {shift}: {e}"));
        let row = rows
            .into_iter()
            .find(|b| b.currency.as_str() == currency)
            .unwrap_or_else(|| panic!("the {currency} balance of {shift}: row not found"));
        (
            row.opening.value().normalize(),
            row.expected.value().normalize(),
            row.counted.map(|m| m.value().normalize()),
        )
    }

    /// `(counted, expected, difference)` of one currency of a count.
    async fn total(
        &self,
        count: &str,
        currency: &str,
    ) -> (Decimal, Option<Decimal>, Option<Decimal>) {
        sqlx::query_as(
            "SELECT counted, expected, difference FROM cash_count_totals \
             WHERE count_id = $1::uuid AND currency = $2",
        )
        .bind(count)
        .bind(currency)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} total of count {count}: {e}"))
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    fn shifts(&self) -> ShiftService {
        ShiftService::new(
            std::sync::Arc::new(
                xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                    self.pool.clone(),
                ),
            ),
            xc_kernel::branch_tx::BranchTx::new(pos_infra::transactions::Transactor::new(
                self.pool.clone(),
            )),
        )
    }
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn cmd(
    branch: &BranchId,
    shift: Option<&str>,
    kind: CountKind,
    lines: Vec<CountLine>,
) -> SubmitCount {
    SubmitCount {
        branch: branch.clone(),
        shift_id: shift.map(str::to_owned),
        kind,
        lines,
        note: None,
        idempotency_key: unique("count"),
    }
}

fn refused_with(result: Result<CashCount, AppError>, expected: CashError) {
    match result {
        Err(AppError::Domain(sentence)) => assert_eq!(sentence, expected.to_string()),
        other => panic!(
            "expected AppError::Domain({:?}), got {other:?}",
            expected.to_string()
        ),
    }
}

/// THB 1000 × 2 + 100 × 3 = 2,300.
async fn thb_2300(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
    ]
}

/// A teller's shift opened the way §3.1 says: the teller counts, a BRANCH_MANAGER verifies.
async fn opened(db: &Db, branch: &BranchId, teller: &Actor, manager: &Actor) -> String {
    let lines = thb_2300(db).await;
    let counts = db.counts();
    let count = counts
        .submit(teller, cmd(branch, None, CountKind::Open, lines), at(0))
        .await
        .expect("the teller submits the OPEN count");
    let verified = counts
        .verify(manager, &count.id, at(60))
        .await
        .expect("the BRANCH_MANAGER verifies the OPEN count");
    verified
        .shift_id
        .expect("a verified OPEN count names the shift it opened")
}

// ── opening a shift ─────────────────────────────────────────────────────────

#[tokio::test]
async fn an_open_count_verified_by_a_branch_manager_opens_the_tellers_shift() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let lines = thb_2300(&db).await;
    let counts = db.counts();

    let count = counts
        .submit(&teller, cmd(&main, None, CountKind::Open, lines), at(0))
        .await
        .expect("the teller submits the OPEN count");
    assert_eq!(count.counted_by, teller.user_id);
    assert_eq!(count.verified_by, None, "waiting until verified (§4.2)");
    assert!(
        db.open_shifts_of(&teller).await.is_empty(),
        "no shift opens before the count is verified (§4.3)"
    );

    let verified = counts
        .verify(&manager, &count.id, at(60))
        .await
        .expect("the BRANCH_MANAGER verifies");
    assert_eq!(verified.verified_by, Some(manager.user_id.clone()));
    let shift = verified.shift_id.expect("the shift it opened");
    assert_eq!(
        db.shift(&shift).await,
        (teller.user_id.clone(), "OPEN".to_owned()),
        "the shift is the counting teller's"
    );
    let (opening, expected, _) = db.balance(&shift, "THB").await;
    assert_eq!(opening, d("2300"), "the shift opens on the verified count");
    assert_eq!(expected, d("2300"));
    let (counted, _, _) = db.total(&count.id, "THB").await;
    assert_eq!(
        counted,
        d("2300"),
        "Σ qty × face_value, computed by the server"
    );
}

#[tokio::test]
async fn a_count_verified_by_its_counter_is_refused_and_opens_nothing() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    // a BRANCH_MANAGER may count (ShiftOperate) and verify (ShiftCloseCertify), but not the same count
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let lines = thb_2300(&db).await;
    let counts = db.counts();
    let count = counts
        .submit(&manager, cmd(&main, None, CountKind::Open, lines), at(0))
        .await
        .expect("the manager submits an OPEN count");

    refused_with(
        counts.verify(&manager, &count.id, at(60)).await,
        CashError::SameCounter,
    );
    assert!(db.open_shifts_of(&manager).await.is_empty());
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM cash_counts WHERE id = '{}' AND verified_by IS NULL",
            count.id
        ))
        .await,
        1,
        "the count is still waiting"
    );
}

#[tokio::test]
async fn a_teller_cannot_verify_an_open_count() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let other = db.person("Nok", &[Role::Teller]).await;
    let lines = thb_2300(&db).await;
    let counts = db.counts();
    let count = counts
        .submit(&teller, cmd(&main, None, CountKind::Open, lines), at(0))
        .await
        .expect("the teller submits the OPEN count");

    refused_with(
        counts.verify(&other, &count.id, at(60)).await,
        CashError::Refused(Refusal::RoleNotAllowed),
    );
    assert!(db.open_shifts_of(&teller).await.is_empty());
}

#[tokio::test]
async fn two_tellers_hold_two_open_shifts_at_one_branch() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let ploy = db.person("Ploy", &[Role::Teller]).await;
    let nok = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let a = opened(&db, &main, &ploy, &manager).await;
    let b = opened(&db, &main, &nok, &manager).await;
    assert_ne!(a, b);
    assert_eq!(
        db.open_shifts_of(&ploy).await,
        vec![(a, main.as_str().to_owned())]
    );
    assert_eq!(
        db.open_shifts_of(&nok).await,
        vec![(b, main.as_str().to_owned())]
    );
}

#[tokio::test]
async fn a_teller_cannot_open_a_second_shift_at_another_branch() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let airport = db.branch("airport").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let first = opened(&db, &main, &teller, &manager).await;

    let lines = thb_2300(&db).await;
    let counts = db.counts();
    let count = counts
        .submit(
            &teller,
            cmd(&airport, None, CountKind::Open, lines),
            at(120),
        )
        .await
        .expect("the OPEN count at the second branch is submitted");
    refused_with(
        counts.verify(&manager, &count.id, at(180)).await,
        CashError::ShiftAlreadyOpen,
    );
    assert_eq!(
        db.open_shifts_of(&teller).await,
        vec![(first, main.as_str().to_owned())],
        "one OPEN shift per teller, across branches (A8)"
    );
}

#[tokio::test]
async fn a_resubmitted_count_with_the_same_idempotency_key_is_the_same_count() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let lines = thb_2300(&db).await;
    let counts = db.counts();
    let first_cmd = cmd(&main, None, CountKind::Open, lines);
    let again = SubmitCount {
        branch: first_cmd.branch.clone(),
        shift_id: first_cmd.shift_id.clone(),
        kind: CountKind::Open,
        lines: first_cmd.lines.clone(),
        note: None,
        idempotency_key: first_cmd.idempotency_key.clone(),
    };
    let one = counts
        .submit(&teller, first_cmd, at(0))
        .await
        .expect("first");
    let two = counts.submit(&teller, again, at(5)).await.expect("again");
    assert_eq!(one.id, two.id);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM cash_counts WHERE branch_id = '{}'",
            main.as_str()
        ))
        .await,
        1
    );
}

// ── closing a shift ─────────────────────────────────────────────────────────

#[tokio::test]
async fn only_the_newest_count_of_a_shift_and_kind_is_verified_and_it_closes_the_shift() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let shift = opened(&db, &main, &teller, &manager).await;
    let counts = db.counts();

    let first = counts
        .submit(
            &teller,
            cmd(&main, Some(&shift), CountKind::Close, thb_2300(&db).await),
            at(600),
        )
        .await
        .expect("a CLOSE count");
    let recount = counts
        .submit(
            &teller,
            cmd(&main, Some(&shift), CountKind::Close, thb_2300(&db).await),
            at(660),
        )
        .await
        .expect("a recount");
    refused_with(
        counts.verify(&manager, &first.id, at(700)).await,
        CashError::NotNewestCount,
    );
    assert_eq!(db.shift(&shift).await.1, "OPEN");

    counts
        .verify(&manager, &recount.id, at(720))
        .await
        .expect("the newest count verifies");
    assert_eq!(db.shift(&shift).await.1, "CLOSED");
    let (_, _, counted) = db.balance(&shift, "THB").await;
    assert_eq!(
        counted,
        Some(d("2300")),
        "the shift closes on the verified count"
    );
}

#[tokio::test]
async fn a_teller_cannot_count_another_tellers_shift() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let ploy = db.person("Ploy", &[Role::Teller]).await;
    let nok = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let shift = opened(&db, &main, &ploy, &manager).await;

    refused_with(
        db.counts()
            .submit(
                &nok,
                cmd(&main, Some(&shift), CountKind::Close, thb_2300(&db).await),
                at(600),
            )
            .await,
        CashError::NotOwnShift,
    );
}

#[tokio::test]
async fn a_close_verify_after_a_later_exchange_on_the_shift_is_refused() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let shift = opened(&db, &main, &teller, &manager).await;
    let counts = db.counts();
    let close = counts
        .submit(
            &teller,
            cmd(&main, Some(&shift), CountKind::Close, thb_2300(&db).await),
            at(600),
        )
        .await
        .expect("a CLOSE count");
    let (_, expected, _) = db.total(&close.id, "THB").await;
    assert_eq!(
        expected,
        Some(d("2300")),
        "a CLOSE count's expected is the shift's expected at the moment of counting (§4.3)"
    );

    // an exchange after the count moves the shift's expected
    let mut tx = db.pool.begin().await.expect("begin");
    lg4_exchange_for_teller(&mut tx, &teller.user_id, "THB", "100").await; // was ShiftService::apply_delta_for_teller_in, "the exchange posts to the teller's shift"
    tx.commit().await.expect("commit");

    refused_with(
        counts.verify(&manager, &close.id, at(700)).await,
        CashError::BalancesChanged,
    );
    assert_eq!(db.shift(&shift).await.1, "OPEN");
}

#[tokio::test]
async fn a_non_zero_difference_writes_one_cash_exception_and_a_zero_one_writes_none() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let ploy = db.person("Ploy", &[Role::Teller]).await;
    let nok = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let short = opened(&db, &main, &ploy, &manager).await;
    let even = opened(&db, &main, &nok, &manager).await;
    let counts = db.counts();

    // V-1 (FINANCE approval above limit) / #928: branch limit covering the -100 THB difference
    sqlx::query(
        "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
         VALUES ($1::uuid, $2, 100.00, 'test', now())",
    )
    .bind(ORG)
    .bind(main.as_str())
    .execute(&db.pool)
    .await
    .expect("insert cash limit");

    // 2,200 counted against 2,300 expected
    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = counts
        .submit(
            &ploy,
            cmd(&main, Some(&short), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("a short CLOSE count");
    counts
        .verify(&manager, &close.id, at(660))
        .await
        .expect("a short count still verifies: a difference is an exception, not a block (C4)");
    assert_eq!(
        db.total(&close.id, "THB").await,
        (d("2200"), Some(d("2300")), Some(d("-100")))
    );
    let exceptions: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT branch_id, data FROM cash_exceptions \
         WHERE branch_id = $1 AND data->>'requestId' = $2",
    )
    .bind(main.as_str())
    .bind(&close.id)
    .fetch_all(&db.pool)
    .await
    .expect("read the exceptions");
    assert_eq!(exceptions.len(), 1, "{exceptions:?}");
    let exc = &exceptions[0].1;
    assert_eq!(exc["status"], "OPEN");
    assert_eq!(exc["note"], format!("COUNT {}", close.id));
    assert_eq!(exc["cashVariance"]["THB"], -10000);
    assert!(exc["safeVariance"].is_null());
    assert_eq!(exc["at"], at(660).timestamp_millis());

    let even_close = counts
        .submit(
            &nok,
            cmd(&main, Some(&even), CountKind::Close, thb_2300(&db).await),
            at(600),
        )
        .await
        .expect("an even CLOSE count");
    counts
        .verify(&manager, &even_close.id, at(660))
        .await
        .expect("an even count verifies");
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM cash_exceptions WHERE data->>'requestId' = '{}'",
            even_close.id
        ))
        .await,
        0
    );
}

// ── an exchange moves the saver's own shift ─────────────────────────────────

#[tokio::test]
async fn an_exchange_posts_to_the_savers_shift_only() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let ploy = db.person("Ploy", &[Role::Teller]).await;
    let nok = db.person("Nok", &[Role::Teller]).await;
    let idle = db.person("Mai", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let a = opened(&db, &main, &ploy, &manager).await;
    let b = opened(&db, &main, &nok, &manager).await;
    let thb_in = StockDelta::for_movement(
        CurrencyCode::parse("THB").unwrap(),
        Direction::In,
        ForeignAmount::from_decimal(Decimal::new(50000, 2)),
    );

    let mut tx = db.pool.begin().await.expect("begin");
    let moved = db
        .shifts()
        .apply_delta_for_teller_in(&mut tx, &main, &nok.user_id, &thb_in)
        .await
        .expect("Nok's exchange");
    lg4_exchange_for_teller(&mut tx, &nok.user_id, "THB", "500").await;
    tx.commit().await.expect("commit");
    assert_eq!(moved, b);
    assert_eq!(db.balance(&b, "THB").await.1, d("2800"));
    assert_eq!(
        db.balance(&a, "THB").await.1,
        d("2300"),
        "Ploy's shift is untouched"
    );

    let mut tx = db.pool.begin().await.expect("begin");
    let none = db
        .shifts()
        .apply_delta_for_teller_in(&mut tx, &main, &idle.user_id, &thb_in)
        .await;
    match none {
        Err(AppError::Domain(sentence)) => {
            assert_eq!(sentence, ShiftError::NoOpenShift.to_string())
        }
        other => panic!("a teller with no OPEN shift: expected noOpenShift, got {other:?}"),
    }
}

// ── amounts (moved here from CA-1, #617) ────────────────────────────────────

#[tokio::test]
async fn a_total_with_more_places_than_the_currency_allows_is_refused_and_nothing_is_written() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    // JPY has minor_unit 0 (RU-1); a 0.5 face value is not in the A7 seed, so the test adds one
    let half: i64 = sqlx::query_scalar(
        "INSERT INTO denominations (currency_code, face_value, group_code) \
         VALUES ('JPY', 0.5, 'ALL') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .expect("a JPY 0.5 face value");
    let lines = vec![CountLine::new("JPY", half, 1)];

    refused_with(
        db.counts()
            .submit(&teller, cmd(&main, None, CountKind::Open, lines), at(0))
            .await,
        CashError::TooManyDecimals,
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM cash_counts WHERE branch_id = '{}'",
            main.as_str()
        ))
        .await,
        0,
        "refused, never rounded (design §4.1 Amounts)"
    );
}

/// LG-4 (#929, lane controller's 1:1 ruling): an OPEN count takes its counted cash out of the branch's SAFE
/// (design §4.4 row 1), so a branch's SAFE is funded through the ledger first. Before the cutover this file needed
/// no funding because nothing was taken from the safe; THB is not inventory, so the funding moves no position.
static LG4_FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

async fn lg4_fund_safe(pool: &PgPool, branch: &str, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(BranchId::new(branch)),
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "lg4-fund:{branch}:{currency}:{}",
                        LG4_FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    ),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund the safe through the ledger");
    tx.commit().await.expect("commit");
}

/// LG-4 (#929, 1:1): after the cutover an exchange moves the drawer through the ledger (TX-5's `post_in`), no
/// longer through `ShiftService::apply_delta_for_teller_in`; this posts the same `currency amount` into the
/// teller's OPEN shift's DRAWER as an `EXCHANGE`, on the caller's transaction.
async fn lg4_exchange_for_teller(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    teller_user_id: &str,
    currency: &str,
    amount: &str,
) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let shift: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'")
            .bind(teller_user_id)
            .fetch_one(&mut **tx)
            .await
            .expect("the teller's OPEN shift");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            tx,
            &[PostingRequest {
                from: CashPlace::Customer,
                to: CashPlace::Drawer(shift.clone()),
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Exchange,
                    source_id: format!("lg4-exchange:{shift}:{currency}:{amount}"),
                    source_key: "complete".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("the exchange posts to the teller's drawer");
}

#[tokio::test]
async fn a_count_with_no_lines_is_refused_and_writes_nothing() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let counts = db.counts();
    let before = db.count("SELECT count(*) FROM cash_counts").await;

    let res = counts
        .submit(&teller, cmd(&main, None, CountKind::Open, vec![]), at(0))
        .await;
    assert!(
        matches!(res, Err(AppError::Validation(_))),
        "expected AppError::Validation(_), got {res:?}"
    );
    assert_eq!(db.count("SELECT count(*) FROM cash_counts").await, before);

    let zero_lines = vec![CountLine::new(
        "THB",
        db.denomination("THB", "1000").await,
        0,
    )];
    counts
        .submit(
            &teller,
            cmd(&main, None, CountKind::Open, zero_lines),
            at(1),
        )
        .await
        .expect("an explicit 0 line is accepted");
}
