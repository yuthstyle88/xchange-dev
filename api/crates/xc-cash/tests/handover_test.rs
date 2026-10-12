//! CA-4 (#620): the hand-over — handing the drawer to the next teller (design
//! `docs/design/blueprint/cash-shift-transfer.md` §3.1 "Hand-over: count / verify", §4.2 `handovers`, §4.3, §9.1 row
//! CA-4: "hand-over closes A's shift and opens B's with the same lines; only B verifies; reject leaves A open").
//!
//! Written before the implementation by claude/83f682, at the lane controller's request (claude/608dff), on top of
//! CA-3's API (#619). Do not edit, delete or weaken these tests; add your own in another file.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.
//!
//! [required] by these tests (proposed on #620, issuecomment-6093020984):
//! - `xc_cash::application::handover::{HandoverService, StartHandover}`:
//!   - `HandoverService::pg(pool: PgPool) -> Self`;
//!   - `start(&self, actor: &Actor, cmd: StartHandover, now: DateTime<Utc>) -> AppResult<Handover>`: the outgoing
//!     teller counts their own shift; it writes one HANDOVER_OUT `cash_counts` row (counted by A, on A's shift, its
//!     `expected` = the shift's expected at that moment) and one PENDING `handovers` row naming the receiver;
//!   - `accept(&self, actor: &Actor, handover_id: &str, now) -> AppResult<Handover>`: only the receiver; in one
//!     transaction it verifies the count (verified_by = the receiver), closes A's shift on the counted lines, opens
//!     B's shift (teller = the receiver, opened by that count) with the same lines, and sets ACCEPTED +
//!     `to_shift_id`;
//!   - `reject(&self, actor: &Actor, handover_id: &str, now) -> AppResult<Handover>`: only the receiver; REJECTED,
//!     the count stays unverified, A's shift stays OPEN.
//!   - `StartHandover { branch: BranchId, from_shift_id: String, receiver_id: String, lines: Vec<CountLine>, note:
//!     Option<String>, idempotency_key: String }`.
//! - `xc_cash::domain::handover::{Handover, HandoverStatus, HandoverError}`:
//!   - `Handover` has the public fields `id: String`, `branch_id: BranchId`, `from_shift_id: String`,
//!     `to_shift_id: Option<String>`, `count_id: String`, `receiver_id: String`, `status: HandoverStatus`;
//!   - `HandoverStatus::{Pending, Accepted, Rejected}` (`Debug`, `PartialEq`);
//!   - `HandoverError::NotPending` (`Display`), for an accept or reject of a hand-over that is no longer PENDING.
//!   - `HandoverError::OnlyThroughHandover` (`Display`): `CashCountService::verify` refuses a HANDOVER_OUT count, so
//!     the receiver rule and the shift moves cannot be skipped (lane controller's ruling, 2026-10-10, on #620's
//!     question 2; this is CA-4's one change to CA-3's `count_service.rs`).
//! - Every refusal reaches the caller as `AppError::Domain(error.to_string())`, the crate's convention (CA-3).
//! - The existing `CashError` variants are CA-2/CA-3's: `SameCounter`, `NotReceiver`, `NotOwnShift`,
//!   `BalancesChanged`, `ShiftAlreadyOpen`.
//!
//! What the tests read from the database: `handovers`, `cash_counts` (kind, counted_by, verified_by, shift_id),
//! `cash_count_totals`, `shifts` (teller_id, status, opened_by_count), `shift_balances` and `cash_exceptions`.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_cash::domain::handover::{Handover, HandoverError, HandoverStatus};
use xc_kernel::ids::BranchId;

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
        let schema = unique("ca4");
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
        sqlx::query_as(
            "SELECT opening, expected, counted FROM shift_balances WHERE shift_id = $1 AND code = $2",
        )
        .bind(shift)
        .bind(currency)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} balance of {shift}: {e}"))
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
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

// ── helpers ─────────────────────────────────────────────────────────────────

impl Db {
    fn handovers(&self) -> HandoverService {
        HandoverService::pg(self.pool.clone())
    }

    /// `(status, from_shift_id, to_shift_id, receiver_id, count_id)` of a hand-over row.
    async fn handover_row(&self, id: &str) -> (String, String, Option<String>, String, String) {
        sqlx::query_as(
            "SELECT status, from_shift_id, to_shift_id, receiver_id::text, count_id::text \
             FROM handovers WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the hand-over {id}: {e}"))
    }

    /// `(kind, counted_by, verified_by, shift_id)` of a count row.
    async fn count_row(&self, id: &str) -> (String, String, Option<String>, Option<String>) {
        sqlx::query_as(
            "SELECT kind, counted_by::text, verified_by::text, shift_id FROM cash_counts WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the count {id}: {e}"))
    }

    async fn opened_by_count(&self, shift: &str) -> Option<String> {
        sqlx::query_scalar("SELECT opened_by_count::text FROM shifts WHERE id = $1")
            .bind(shift)
            .fetch_one(&self.pool)
            .await
            .expect("read opened_by_count")
    }

    async fn handover_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM handovers").await
    }
}

fn refused_with<T: std::fmt::Debug>(result: Result<T, AppError>, expected: &str) {
    match result {
        Err(AppError::Domain(sentence)) => assert_eq!(sentence, expected),
        other => panic!("expected AppError::Domain({expected:?}), got {other:?}"),
    }
}

/// THB 1000 × 2 + 100 × 3 = 2,300.
async fn thb_2300(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
    ]
}

/// THB 1000 × 2 + 100 × 2 = 2,200 (100 short of 2,300).
async fn thb_2200(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ]
}

/// A teller's shift opened the way CA-3 opens it: the teller counts OPEN, a BRANCH_MANAGER verifies.
async fn opened(db: &Db, branch: &BranchId, teller: &Actor, manager: &Actor) -> String {
    let counts = db.counts();
    let count = counts
        .submit(
            teller,
            SubmitCount {
                branch: branch.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: thb_2300(db).await,
                note: None,
                idempotency_key: unique("open"),
            },
            at(0),
        )
        .await
        .expect("the teller submits the OPEN count");
    counts
        .verify(manager, &count.id, at(60))
        .await
        .expect("the BRANCH_MANAGER verifies the OPEN count")
        .shift_id
        .expect("a verified OPEN count names the shift it opened")
}

fn start_cmd(
    branch: &BranchId,
    from_shift: &str,
    receiver: &Actor,
    lines: Vec<CountLine>,
) -> StartHandover {
    StartHandover {
        branch: branch.clone(),
        from_shift_id: from_shift.to_owned(),
        receiver_id: receiver.user_id.clone(),
        lines,
        note: None,
        idempotency_key: unique("handover"),
    }
}

/// One branch, A (Ploy) holding an open shift of THB 2,300, B (Nok) the receiver, a manager (Krit) and a third
/// teller (Mai).
struct Floor {
    db: Db,
    main: BranchId,
    a: Actor,
    b: Actor,
    manager: Actor,
    third: Actor,
    a_shift: String,
}

async fn floor() -> Floor {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let third = db.person("Mai", &[Role::Teller]).await;
    let a_shift = opened(&db, &main, &a, &manager).await;
    Floor {
        db,
        main,
        a,
        b,
        manager,
        third,
        a_shift,
    }
}

async fn started(f: &Floor) -> Handover {
    f.db.handovers()
        .start(
            &f.a,
            start_cmd(&f.main, &f.a_shift, &f.b, thb_2300(&f.db).await),
            at(600),
        )
        .await
        .expect("A starts the hand-over of their own shift to B")
}

// ── start ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn start_writes_one_handover_out_count_and_one_pending_handover_and_moves_nothing() {
    let f = floor().await;
    let h = started(&f).await;

    assert_eq!(h.status, HandoverStatus::Pending);
    assert_eq!(h.from_shift_id, f.a_shift);
    assert_eq!(h.to_shift_id, None);
    assert_eq!(h.receiver_id, f.b.user_id);
    assert_eq!(h.branch_id, f.main);
    assert_eq!(
        f.db.handover_row(&h.id).await,
        (
            "PENDING".to_owned(),
            f.a_shift.clone(),
            None,
            f.b.user_id.clone(),
            h.count_id.clone()
        )
    );
    assert_eq!(
        f.db.count_row(&h.count_id).await,
        (
            "HANDOVER_OUT".to_owned(),
            f.a.user_id.clone(),
            None,
            Some(f.a_shift.clone())
        ),
        "the count is A's, of A's shift, waiting (§3.1, §4.2)"
    );
    assert_eq!(
        f.db.total(&h.count_id, "THB").await,
        (d("2300"), Some(d("2300")), None),
        "counted Σ qty × face value; expected = A's shift at the moment of counting (§4.3)"
    );
    assert_eq!(
        f.db.shift(&f.a_shift).await.1,
        "OPEN",
        "A keeps the drawer until B accepts"
    );
    assert!(
        f.db.open_shifts_of(&f.b).await.is_empty(),
        "B has no shift yet"
    );
}

#[tokio::test]
async fn only_the_shifts_own_teller_can_start_its_handover() {
    let f = floor().await;
    for (who, label) in [
        (&f.third, "another teller"),
        (&f.manager, "a BRANCH_MANAGER"),
    ] {
        refused_with(
            f.db.handovers()
                .start(
                    who,
                    start_cmd(&f.main, &f.a_shift, &f.b, thb_2300(&f.db).await),
                    at(600),
                )
                .await,
            &CashError::NotOwnShift.to_string(),
        );
        assert_eq!(f.db.handover_rows().await, 0, "{label}: no hand-over row");
    }
    assert_eq!(
        f.db.count("SELECT count(*) FROM cash_counts WHERE kind = 'HANDOVER_OUT'")
            .await,
        0,
        "no HANDOVER_OUT count either"
    );
}

#[tokio::test]
async fn a_teller_cannot_hand_over_to_themselves() {
    let f = floor().await;
    refused_with(
        f.db.handovers()
            .start(
                &f.a,
                start_cmd(&f.main, &f.a_shift, &f.a, thb_2300(&f.db).await),
                at(600),
            )
            .await,
        &CashError::SameCounter.to_string(),
    );
    assert_eq!(f.db.handover_rows().await, 0);
    assert_eq!(f.db.shift(&f.a_shift).await.1, "OPEN");
}

// ── accept ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_receiver_accepts_a_shift_closes_and_b_shift_opens_with_the_same_lines() {
    let f = floor().await;
    let h = started(&f).await;

    let accepted =
        f.db.handovers()
            .accept(&f.b, &h.id, at(660))
            .await
            .expect("B, the receiver, accepts");

    assert_eq!(accepted.status, HandoverStatus::Accepted);
    let b_shift = accepted
        .to_shift_id
        .clone()
        .expect("the accepted hand-over names B's new shift");
    assert_ne!(b_shift, f.a_shift);
    assert_eq!(
        f.db.handover_row(&h.id).await,
        (
            "ACCEPTED".to_owned(),
            f.a_shift.clone(),
            Some(b_shift.clone()),
            f.b.user_id.clone(),
            h.count_id.clone()
        )
    );
    assert_eq!(
        f.db.count_row(&h.count_id).await.2,
        Some(f.b.user_id.clone()),
        "the count is verified by B (§3.1: the receiving TELLER)"
    );

    // A's shift closed on the counted lines
    assert_eq!(
        f.db.shift(&f.a_shift).await,
        (f.a.user_id.clone(), "CLOSED".to_owned())
    );
    let (_, _, a_counted) = f.db.balance(&f.a_shift, "THB").await;
    assert_eq!(
        a_counted,
        Some(d("2300")),
        "A's shift is closed on the hand-over count"
    );

    // B's shift opened with the same lines, by that count
    assert_eq!(
        f.db.shift(&b_shift).await,
        (f.b.user_id.clone(), "OPEN".to_owned())
    );
    assert_eq!(
        f.db.open_shifts_of(&f.b).await,
        vec![(b_shift.clone(), f.main.as_str().to_owned())]
    );
    let (opening, expected, _) = f.db.balance(&b_shift, "THB").await;
    assert_eq!(
        (opening, expected),
        (d("2300"), d("2300")),
        "B opens with the same lines"
    );
    assert_eq!(
        f.db.opened_by_count(&b_shift).await,
        Some(h.count_id.clone()),
        "B's shift is opened by the verified hand-over count (§4.1 opened_by_count, §4.3)"
    );
    assert!(
        f.db.open_shifts_of(&f.a).await.is_empty(),
        "A holds no open shift"
    );
}

#[tokio::test]
async fn only_the_receiver_can_accept_and_a_refusal_changes_nothing() {
    let f = floor().await;
    let h = started(&f).await;
    for (who, label) in [
        (&f.third, "a third teller"),
        (&f.manager, "a BRANCH_MANAGER"),
        (&f.a, "the counter"),
    ] {
        refused_with(
            f.db.handovers().accept(who, &h.id, at(660)).await,
            &CashError::NotReceiver.to_string(),
        );
        assert_eq!(f.db.handover_row(&h.id).await.0, "PENDING", "{label}");
    }
    assert_eq!(
        f.db.count_row(&h.count_id).await.2,
        None,
        "the count is still waiting"
    );
    assert_eq!(f.db.shift(&f.a_shift).await.1, "OPEN");
    assert!(f.db.open_shifts_of(&f.b).await.is_empty());
    assert!(f.db.open_shifts_of(&f.third).await.is_empty());
}

#[tokio::test]
async fn a_short_handover_count_opens_b_on_the_counted_lines_and_writes_one_exception() {
    let f = floor().await;
    let h =
        f.db.handovers()
            .start(
                &f.a,
                start_cmd(&f.main, &f.a_shift, &f.b, thb_2200(&f.db).await),
                at(600),
            )
            .await
            .expect("A counts 2,200 against 2,300 expected");
    let accepted =
        f.db.handovers()
            .accept(&f.b, &h.id, at(660))
            .await
            .expect("B accepts");
    let b_shift = accepted.to_shift_id.expect("B's shift");

    let (opening, expected, _) = f.db.balance(&b_shift, "THB").await;
    assert_eq!(
        (opening, expected),
        (d("2200"), d("2200")),
        "B takes what was counted, not what was expected"
    );
    assert_eq!(
        f.db.total(&h.count_id, "THB").await,
        (d("2200"), Some(d("2300")), Some(d("-100")))
    );
    let exceptions: Vec<(String, String)> = sqlx::query_as(
        "SELECT data->>'currency', data->>'difference' FROM cash_exceptions \
         WHERE branch_id = $1 AND data->>'countId' = $2",
    )
    .bind(f.main.as_str())
    .bind(&h.count_id)
    .fetch_all(&f.db.pool)
    .await
    .expect("read the exceptions");
    assert_eq!(
        exceptions.len(),
        1,
        "a difference is an exception, not a block (C4): {exceptions:?}"
    );
    assert_eq!(exceptions[0].0, "THB");
    assert_eq!(d(&exceptions[0].1), d("-100"));
}

#[tokio::test]
async fn an_exchange_on_a_shift_after_the_count_refuses_the_accept() {
    let f = floor().await;
    let h = started(&f).await;

    let mut tx = f.db.pool.begin().await.expect("begin");
    lg4_exchange_for_teller(&mut tx, &f.a.user_id, "THB", "100").await; // was ShiftService::apply_delta_for_teller_in, "an exchange posts to A's shift after the count"
    tx.commit().await.expect("commit");

    refused_with(
        f.db.handovers().accept(&f.b, &h.id, at(700)).await,
        &CashError::BalancesChanged.to_string(),
    );
    assert_eq!(f.db.handover_row(&h.id).await.0, "PENDING");
    assert_eq!(f.db.shift(&f.a_shift).await.1, "OPEN");
    assert!(f.db.open_shifts_of(&f.b).await.is_empty());
}

#[tokio::test]
async fn a_receiver_who_already_holds_an_open_shift_cannot_accept() {
    let f = floor().await;
    let _b_own = opened(&f.db, &f.main, &f.b, &f.manager).await;
    let h = started(&f).await;

    refused_with(
        f.db.handovers().accept(&f.b, &h.id, at(660)).await,
        &CashError::ShiftAlreadyOpen.to_string(),
    );
    assert_eq!(f.db.handover_row(&h.id).await.0, "PENDING");
    assert_eq!(f.db.count_row(&h.count_id).await.2, None);
    assert_eq!(
        f.db.shift(&f.a_shift).await.1,
        "OPEN",
        "A keeps the drawer (A8)"
    );
    assert_eq!(
        f.db.open_shifts_of(&f.b).await.len(),
        1,
        "B still holds only their own shift"
    );
}

#[tokio::test]
async fn an_accepted_handover_cannot_be_accepted_or_rejected_again() {
    let f = floor().await;
    let h = started(&f).await;
    f.db.handovers()
        .accept(&f.b, &h.id, at(660))
        .await
        .expect("B accepts");

    refused_with(
        f.db.handovers().accept(&f.b, &h.id, at(700)).await,
        &HandoverError::NotPending.to_string(),
    );
    refused_with(
        f.db.handovers().reject(&f.b, &h.id, at(700)).await,
        &HandoverError::NotPending.to_string(),
    );
    assert_eq!(f.db.handover_row(&h.id).await.0, "ACCEPTED");
    assert_eq!(
        f.db.open_shifts_of(&f.b).await.len(),
        1,
        "no second shift for B"
    );
}

// ── reject ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_receiver_rejects_and_a_keeps_the_drawer() {
    let f = floor().await;
    let h = started(&f).await;

    let rejected =
        f.db.handovers()
            .reject(&f.b, &h.id, at(660))
            .await
            .expect("B, the receiver, rejects");

    assert_eq!(rejected.status, HandoverStatus::Rejected);
    assert_eq!(rejected.to_shift_id, None);
    assert_eq!(f.db.handover_row(&h.id).await.0, "REJECTED");
    assert_eq!(
        f.db.count_row(&h.count_id).await.2,
        None,
        "a rejected count is not verified"
    );
    assert_eq!(
        f.db.shift(&f.a_shift).await.1,
        "OPEN",
        "a reject leaves A open (§9.1)"
    );
    let (_, expected, counted) = f.db.balance(&f.a_shift, "THB").await;
    assert_eq!(
        (expected, counted),
        (d("2300"), None),
        "A's balances untouched"
    );
    assert!(f.db.open_shifts_of(&f.b).await.is_empty());

    refused_with(
        f.db.handovers().accept(&f.b, &h.id, at(700)).await,
        &HandoverError::NotPending.to_string(),
    );
}

#[tokio::test]
async fn only_the_receiver_can_reject() {
    let f = floor().await;
    let h = started(&f).await;
    for who in [&f.third, &f.manager, &f.a] {
        refused_with(
            f.db.handovers().reject(who, &h.id, at(660)).await,
            &CashError::NotReceiver.to_string(),
        );
    }
    assert_eq!(f.db.handover_row(&h.id).await.0, "PENDING");
}

#[tokio::test]
async fn after_a_reject_a_can_hand_over_again_and_the_new_one_is_accepted() {
    let f = floor().await;
    let first = started(&f).await;
    f.db.handovers()
        .reject(&f.b, &first.id, at(660))
        .await
        .expect("B rejects");

    let second =
        f.db.handovers()
            .start(
                &f.a,
                start_cmd(&f.main, &f.a_shift, &f.third, thb_2300(&f.db).await),
                at(700),
            )
            .await
            .expect("A hands over again, to Mai");
    assert_ne!(second.count_id, first.count_id, "a new HANDOVER_OUT count");
    let accepted =
        f.db.handovers()
            .accept(&f.third, &second.id, at(760))
            .await
            .expect("Mai accepts");
    let mai_shift = accepted.to_shift_id.expect("Mai's shift");
    assert_eq!(
        f.db.shift(&mai_shift).await,
        (f.third.user_id.clone(), "OPEN".to_owned())
    );
    assert_eq!(f.db.shift(&f.a_shift).await.1, "CLOSED");
}

/// #619 must-fix 4 (lane controller's ruling, #619 issuecomment-6093070975): a count covers every currency the shift
/// holds, and an omitted one counts as 0. A hand-over count that leaves out USD therefore hands B no USD, and A's
/// missing USD is an exception.
#[tokio::test]
async fn a_currency_a_holds_but_the_handover_count_omits_reads_as_zero() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let mut open_lines = thb_2300(&db).await;
    open_lines.push(CountLine::new(
        "USD",
        db.denomination("USD", "100").await,
        1,
    ));
    let counts = db.counts();
    let open = counts
        .submit(
            &a,
            SubmitCount {
                branch: main.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: open_lines,
                note: None,
                idempotency_key: unique("open"),
            },
            at(0),
        )
        .await
        .expect("A opens with THB 2,300 and USD 100");
    // LG-4 (#929, 1:1): the OPEN count takes its USD 100 out of the safe, so the safe holds it first.
    lg4_fund_safe(&db.pool, main.as_str(), "USD", "100.00").await;
    let a_shift = counts
        .verify(&manager, &open.id, at(60))
        .await
        .expect("the manager verifies")
        .shift_id
        .expect("A's shift");

    let h = db
        .handovers()
        .start(
            &a,
            start_cmd(&main, &a_shift, &b, thb_2300(&db).await),
            at(600),
        )
        .await
        .expect("A counts THB only");
    let b_shift = db
        .handovers()
        .accept(&b, &h.id, at(660))
        .await
        .expect("B accepts")
        .to_shift_id
        .expect("B's shift");

    assert_eq!(db.balance(&b_shift, "THB").await.0, d("2300"));
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM shift_balances WHERE shift_id = '{b_shift}' AND code = 'USD' AND opening <> 0"
        ))
        .await,
        0,
        "B is handed no USD"
    );
    let exceptions: Vec<(String, String)> = sqlx::query_as(
        "SELECT data->>'currency', data->>'difference' FROM cash_exceptions \
         WHERE branch_id = $1 AND data->>'countId' = $2",
    )
    .bind(main.as_str())
    .bind(&h.count_id)
    .fetch_all(&db.pool)
    .await
    .expect("read the exceptions");
    assert_eq!(exceptions.len(), 1, "{exceptions:?}");
    assert_eq!(exceptions[0].0, "USD");
    assert_eq!(d(&exceptions[0].1), d("-100"));
}

/// #620 question 2 (lane controller's ruling, 2026-10-10): a HANDOVER_OUT count is verified only through
/// `HandoverService::accept`. The generic `CashCountService::verify` refuses it, even for the receiver, so neither
/// the receiver rule nor the shift moves can be skipped.
#[tokio::test]
async fn the_generic_count_verify_refuses_a_handover_out_count() {
    let f = floor().await;
    let h = started(&f).await;
    for who in [&f.b, &f.manager, &f.third] {
        refused_with(
            f.db.counts().verify(who, &h.count_id, at(660)).await,
            &HandoverError::OnlyThroughHandover.to_string(),
        );
    }
    assert_eq!(
        f.db.count_row(&h.count_id).await.2,
        None,
        "the count is still waiting"
    );
    assert_eq!(f.db.handover_row(&h.id).await.0, "PENDING");
    assert_eq!(f.db.shift(&f.a_shift).await.1, "OPEN");
    assert!(f.db.open_shifts_of(&f.b).await.is_empty());

    f.db.handovers()
        .accept(&f.b, &h.id, at(700))
        .await
        .expect("the hand-over path still accepts it");
}

// ── the CLOSE verify gap CA-4 closes while it changes `verify` ─────────────────

/// Lane controller's note on #620 (2026-10-10T07:18Z, from the PR 871 review): at CLOSE, a currency the
/// shift holds but the count has no total for — first held **after** `submit`, because a later exchange
/// added it — must be refused `BalancesChanged` at `verify`, and nothing is written. On main (`6524a80`)
/// `verify` adds such a currency with its current `expected`, so it passes the check and its difference
/// is written. CA-4 changes `verify` (the HANDOVER_OUT refusal), so it closes this case too.
/// Added by the ticket author (claude/14cc24bd) on the lane controller's order; the 14 tests above are
/// unchanged.
#[tokio::test]
async fn a_close_verify_refuses_a_currency_first_held_after_the_count_and_writes_nothing() {
    let f = floor().await;
    let close =
        f.db.counts()
            .submit(
                &f.a,
                SubmitCount {
                    branch: f.main.clone(),
                    shift_id: Some(f.a_shift.clone()),
                    kind: CountKind::Close,
                    lines: thb_2300(&f.db).await,
                    note: None,
                    idempotency_key: unique("close"),
                },
                at(600),
            )
            .await
            .expect("A submits a CLOSE count holding THB only");
    let exceptions_before = f.db.count("SELECT count(*) FROM cash_exceptions").await;

    // an exchange after the count adds a currency the count has no total for
    let mut tx = f.db.pool.begin().await.expect("begin");
    lg4_exchange_for_teller(&mut tx, &f.a.user_id, "USD", "100").await; // was ShiftService::apply_delta_for_teller_in, "an exchange posts USD to A's shift after the count"
    tx.commit().await.expect("commit");

    refused_with(
        f.db.counts().verify(&f.manager, &close.id, at(700)).await,
        &CashError::BalancesChanged.to_string(),
    );
    assert_eq!(
        f.db.count_row(&close.id).await.2,
        None,
        "the count is still waiting"
    );
    assert_eq!(
        f.db.shift(&f.a_shift).await.1,
        "OPEN",
        "the shift is not closed"
    );
    assert_eq!(
        f.db.count("SELECT count(*) FROM cash_exceptions").await,
        exceptions_before,
        "no difference is written"
    );
    assert_eq!(
        f.db.count(&format!(
            "SELECT count(*) FROM cash_count_totals WHERE count_id = '{}' AND currency = 'USD'",
            close.id
        ))
        .await,
        0,
        "no total is added for the new currency"
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
