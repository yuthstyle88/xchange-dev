//! Ticket #929 (LG-4) — the cutover, as counts and hand-overs see it (design
//! `docs/design/blueprint/ledger-inventory.md` §4.2, §4.4 rows 1, 2 and 7, L-4; lane controller's rulings on #929:
//! commit 2 = OPENING and the hand-over, commit 3 = the count difference).
//! Written before the implementation by the ticket author (claude/89bd61d9, rule A). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! What this test pins ([required] unless marked):
//! - **Commit 2.**
//!   - An OPEN count verified posts `SAFE(branch) → DRAWER(shift)` of the counted lines, `source_type OPENING`,
//!     `source_id` = the count's id, on `verify_in`'s transaction: the new drawer holds what was counted and the
//!     safe holds that much less.
//!   - The safe cannot cover it ⇒ the verify is refused (`stockInsufficient`, the port's refusal, §4.5) and no shift
//!     opens.
//!   - A hand-over accepted posts `DRAWER(from) → DRAWER(to)` of the hand-over count's lines, `source_type OPENING`,
//!     `source_id` = the hand-over's id, on `accept_in`'s transaction (§4.2: "the hand-over's DRAWER(previous) →
//!     DRAWER(shift)").
//! - **Commit 3.**
//!   - A count's `expected` is the drawer's balance **in the ledger** (L-4: balances are derived from the ledger), on
//!     `submit_in` and on `verify_in`'s re-check: an exchange that moved only the ledger drawer shows in it.
//!   - A verified CLOSE count's `counted − expected` posts `DRAWER ↔ VARIANCE(branch)`: short ⇒ `DRAWER → VARIANCE`,
//!     over ⇒ `VARIANCE → DRAWER`; a foreign difference is an inventory `ADJUST` at the position's average, never a
//!     `DISPOSE` or `ACQUIRE` (§4.4 row 7). `VARIANCE` is read with SQL here, so this file compiles before
//!     `CashPlace::Variance` exists.
//!
//! Each branch has a generous `cash_limits` row so a difference is verified without FINANCE (LG-5's gate is not what
//! this file tests). Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

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
    Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).unwrap()
}

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
        let schema = unique("lg4c");
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
            .expect("every migration applies");
        Db { pool }
    }

    async fn post(&self, reqs: Vec<PostingRequest>) {
        let mut tx = self.pool.begin().await.unwrap();
        LedgerCashPostings::new()
            .post_in(&mut tx, &reqs, None)
            .await
            .unwrap_or_else(|e| panic!("seed posting: {e:?}"));
        tx.commit().await.unwrap();
    }

    fn src(key: &str, kind: SourceType) -> SourceRef {
        SourceRef {
            source_type: kind,
            source_id: unique("seed"),
            source_key: key.to_string(),
        }
    }

    /// A branch with THB `thb` in its SAFE (ledger), and USD `usd` bought at 32.10 (so its position has a cost), and
    /// a generous count-difference limit.
    async fn branch(&self, label: &str, thb: &str, usd: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, 1000000, 'test', now())",
        )
        .bind(ORG)
        .bind(&id)
        .execute(&self.pool)
        .await
        .expect("a generous count-difference limit");
        let b = BranchId::new(id);
        if d(thb) > Decimal::ZERO {
            self.post(vec![PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(b.clone()),
                currency: CurrencyCode::THB,
                quantity: ForeignAmount::from_decimal(d(thb)),
                source: Self::src("thb", SourceType::Opening),
                line: None,
                reverses: None,
            }])
            .await;
        }
        if d(usd) > Decimal::ZERO {
            let s = Self::src("usd", SourceType::Opening);
            self.post(vec![
                PostingRequest {
                    from: CashPlace::Customer,
                    to: CashPlace::Safe(b.clone()),
                    currency: code("USD"),
                    quantity: ForeignAmount::from_decimal(d(usd)),
                    source: s.clone(),
                    line: None,
                    reverses: None,
                },
                PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Customer,
                    currency: CurrencyCode::THB,
                    quantity: ForeignAmount::from_decimal(d(usd) * d("32.10")),
                    source: s,
                    line: None,
                    reverses: None,
                },
            ])
            .await;
        }
        b
    }

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

    async fn face(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    /// A place's balance of `currency` through the port.
    async fn at(&self, place: CashPlace, currency: &str) -> Decimal {
        let mut tx = self.pool.begin().await.unwrap();
        let b = LedgerCashPostings::new()
            .expected_in(&mut tx, &place)
            .await
            .expect("expected_in");
        tx.rollback().await.unwrap();
        Decimal::new(b.get(code(currency)).minor(), 2)
    }

    async fn dec(&self, sql: &str) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .unwrap_or_default()
    }

    async fn int(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// The `VARIANCE` balance of `currency` at `branch` (SQL: the account kind, before the port has the place).
    async fn variance(&self, branch: &BranchId, currency: &str) -> Decimal {
        self.dec(&format!(
            "SELECT sum(e.quantity_signed) FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
             WHERE a.kind = 'VARIANCE' AND a.branch_id = '{}' AND trim(a.currency) = '{currency}'",
            branch.as_str()
        ))
        .await
    }
}

async fn thb_2300(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.face("THB", "1000").await, 2),
        CountLine::new("THB", db.face("THB", "100").await, 3),
    ]
}

fn open_cmd(branch: &BranchId, lines: Vec<CountLine>) -> SubmitCount {
    SubmitCount {
        branch: branch.clone(),
        shift_id: None,
        kind: CountKind::Open,
        lines,
        note: None,
        idempotency_key: unique("open"),
    }
}

fn close_cmd(branch: &BranchId, shift: &str, lines: Vec<CountLine>) -> SubmitCount {
    SubmitCount {
        branch: branch.clone(),
        shift_id: Some(shift.to_owned()),
        kind: CountKind::Close,
        lines,
        note: None,
        idempotency_key: unique("close"),
    }
}

/// `teller` opens a shift counting `lines`; `manager` verifies. Answers (count id, shift id).
async fn opened(
    db: &Db,
    branch: &BranchId,
    teller: &Actor,
    manager: &Actor,
    lines: Vec<CountLine>,
) -> (String, String) {
    let counts = db.counts();
    let count = counts
        .submit(teller, open_cmd(branch, lines), at(0))
        .await
        .expect("the teller submits the OPEN count");
    let shift = counts
        .verify(manager, &count.id, at(60))
        .await
        .expect("the BRANCH_MANAGER verifies the OPEN count")
        .shift_id
        .expect("the shift it opened");
    (count.id, shift)
}

// ── commit 2: the OPEN count's OPENING (§4.4 row 1) ──────────────────────────

#[tokio::test]
async fn an_open_count_verified_moves_the_counted_cash_from_the_safe_into_the_new_drawer() {
    let db = Db::fresh().await;
    let main = db.branch("main", "10000.00", "0").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (count, shift) = opened(&db, &main, &teller, &manager, thb_2300(&db).await).await;

    assert_eq!(
        db.at(CashPlace::Drawer(shift.clone()), "THB").await,
        d("2300.00"),
        "the new drawer holds what was counted"
    );
    assert_eq!(
        db.at(CashPlace::Safe(main.clone()), "THB").await,
        d("7700.00"),
        "the safe holds that much less"
    );
    assert_eq!(
        db.int(&format!(
            "SELECT count(*) FROM ledger_postings WHERE source_type = 'OPENING' AND source_id = '{count}'"
        ))
        .await,
        1,
        "one OPENING posting named by the count"
    );
}

#[tokio::test]
async fn an_open_count_the_safe_cannot_cover_is_refused_and_opens_nothing() {
    let db = Db::fresh().await;
    let main = db.branch("main", "1000.00", "0").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let counts = db.counts();
    let count = counts
        .submit(&teller, open_cmd(&main, thb_2300(&db).await), at(0))
        .await
        .expect("submit");
    let refused = counts.verify(&manager, &count.id, at(60)).await;
    assert!(
        matches!(&refused, Err(e) if format!("{e:?}").contains("stockInsufficient")),
        "the safe holds THB 1,000 and the count takes 2,300: {refused:?}"
    );
    assert_eq!(
        db.int(&format!(
            "SELECT count(*) FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller.user_id
        ))
        .await,
        0,
        "no shift opens"
    );
    assert_eq!(
        db.at(CashPlace::Safe(main), "THB").await,
        d("1000.00"),
        "the safe is untouched"
    );
}

/// Option A (lane controller, issuecomment-6115055843): a THB+USD OPEN count is **one** OPENING posting named by
/// the count, holding one entry pair (SAFE −, DRAWER +) per currency; each currency's drawer equals what was counted.
#[tokio::test]
async fn a_two_currency_open_count_is_one_opening_posting_with_an_entry_pair_per_currency() {
    let db = Db::fresh().await;
    let main = db.branch("main", "10000.00", "100.00").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let mut lines = thb_2300(&db).await;
    lines.push(CountLine::new("USD", db.face("USD", "100").await, 1));
    let (count, shift) = opened(&db, &main, &teller, &manager, lines).await;

    assert_eq!(
        db.at(CashPlace::Drawer(shift.clone()), "THB").await,
        d("2300.00")
    );
    assert_eq!(db.at(CashPlace::Drawer(shift), "USD").await, d("100.00"));
    assert_eq!(db.at(CashPlace::Safe(main.clone()), "USD").await, d("0.00"));
    assert_eq!(
        db.int(&format!(
            "SELECT count(*) FROM ledger_postings WHERE source_type = 'OPENING' AND source_id = '{count}'"
        ))
        .await,
        1,
        "one OPENING posting for the whole count"
    );
    let pairs: Vec<(String, i64)> = sqlx::query_as(
        "SELECT trim(e.currency), count(*) FROM ledger_entries e JOIN ledger_postings p ON p.id = e.posting_id \
         WHERE p.source_type = 'OPENING' AND p.source_id = $1 GROUP BY 1 ORDER BY 1",
    )
    .bind(&count)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        pairs,
        vec![("THB".to_owned(), 2), ("USD".to_owned(), 2)],
        "one entry pair per currency"
    );
}

// ── commit 2: the hand-over (§4.4 row 2) ─────────────────────────────────────

#[tokio::test]
async fn an_accepted_handover_moves_the_counted_cash_from_drawer_to_drawer() {
    let db = Db::fresh().await;
    let main = db.branch("main", "10000.00", "0").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (_, a_shift) = opened(&db, &main, &a, &manager, thb_2300(&db).await).await;
    let handovers = HandoverService::pg(db.pool.clone());
    let lines = vec![
        CountLine::new("THB", db.face("THB", "1000").await, 2),
        CountLine::new("THB", db.face("THB", "100").await, 2),
    ]; // 2,200
    let h = handovers
        .start(
            &a,
            StartHandover {
                branch: main.clone(),
                from_shift_id: a_shift.clone(),
                receiver_id: b.user_id.clone(),
                lines,
                note: None,
                idempotency_key: unique("handover"),
            },
            at(600),
        )
        .await
        .expect("A starts the hand-over");
    let h = handovers
        .accept(&b, &h.id, at(700))
        .await
        .expect("B accepts");
    let b_shift: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'")
            .bind(&b.user_id)
            .fetch_one(&db.pool)
            .await
            .expect("B's new shift");
    assert_eq!(
        db.at(CashPlace::Drawer(b_shift), "THB").await,
        d("2200.00"),
        "B's drawer holds what the hand-over counted"
    );
    assert_eq!(
        db.at(CashPlace::Drawer(a_shift), "THB").await,
        d("100.00"),
        "exactly 2,200 left A's drawer"
    );
    assert_eq!(
        db.int(&format!(
            "SELECT count(*) FROM ledger_postings WHERE source_type = 'OPENING' AND source_id = '{}'",
            h.id
        ))
        .await,
        1,
        "one OPENING posting named by the hand-over"
    );
}

// ── commit 3: the count's expected and its difference (§4.4 row 7, L-4) ──────

#[tokio::test]
async fn a_close_counts_expected_is_the_ledger_drawer() {
    let db = Db::fresh().await;
    let main = db.branch("main", "10000.00", "0").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (_, shift) = opened(&db, &main, &teller, &manager, thb_2300(&db).await).await;
    // an exchange after the cutover moves only the ledger drawer (TX-5's post_in)
    db.post(vec![PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift.clone()),
        currency: CurrencyCode::THB,
        quantity: ForeignAmount::from_decimal(d("500.00")),
        source: Db::src("complete", SourceType::Exchange),
        line: None,
        reverses: None,
    }])
    .await;
    let close = db
        .counts()
        .submit(
            &teller,
            close_cmd(&main, &shift, thb_2300(&db).await),
            at(600),
        )
        .await
        .expect("a CLOSE count");
    assert_eq!(
        db.dec(&format!(
            "SELECT expected FROM cash_count_totals WHERE count_id = '{}'::uuid AND trim(currency) = 'THB'",
            close.id
        ))
        .await
        .normalize(),
        d("2800"),
        "expected = the ledger drawer (2,300 opened + 500 exchanged)"
    );
}

#[tokio::test]
async fn a_short_close_count_puts_the_difference_on_variance_and_adjusts_at_average() {
    let db = Db::fresh().await;
    let main = db.branch("main", "10000.00", "100.00").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let mut open_lines = thb_2300(&db).await;
    open_lines.push(CountLine::new("USD", db.face("USD", "100").await, 1));
    let (_, shift) = opened(&db, &main, &teller, &manager, open_lines).await;
    let mut close_lines = thb_2300(&db).await;
    close_lines.push(CountLine::new("USD", db.face("USD", "50").await, 1));
    close_lines.push(CountLine::new("USD", db.face("USD", "20").await, 2)); // USD 90: 10 short
    let counts = db.counts();
    let close = counts
        .submit(&teller, close_cmd(&main, &shift, close_lines), at(600))
        .await
        .expect("a CLOSE count");
    counts
        .verify(&manager, &close.id, at(660))
        .await
        .expect("verified");

    assert_eq!(
        db.variance(&main, "USD").await,
        d("10"),
        "USD 10 short is on the branch's VARIANCE"
    );
    assert_eq!(
        db.at(CashPlace::Drawer(shift), "USD").await,
        d("90.00"),
        "the drawer holds what was counted"
    );
    let posting = format!(
        "SELECT p.id FROM ledger_postings p JOIN ledger_entries e ON e.posting_id = p.id \
         JOIN cash_accounts a ON a.id = e.account_id WHERE a.kind = 'VARIANCE' AND a.branch_id = '{}'",
        main.as_str()
    );
    assert_eq!(
        db.int(&format!(
            "SELECT count(*) FROM inventory_movements WHERE posting_id IN ({posting}) \
             AND movement_type IN ('DISPOSE', 'ACQUIRE')"
        ))
        .await,
        0,
        "no DISPOSE or ACQUIRE for a count difference"
    );
    assert_eq!(
        db.dec(&format!(
            "SELECT sum(quantity_signed) FROM inventory_movements WHERE posting_id IN ({posting}) \
             AND movement_type = 'ADJUST'"
        ))
        .await,
        d("-10"),
        "an ADJUST of −10"
    );
    assert_eq!(
        db.dec(&format!(
            "SELECT sum(cost_thb_signed) FROM inventory_movements WHERE posting_id IN ({posting}) \
             AND movement_type = 'ADJUST'"
        ))
        .await,
        d("-321.00"),
        "at the position's average 32.10"
    );
}

#[tokio::test]
async fn an_over_close_count_takes_the_difference_out_of_variance() {
    let db = Db::fresh().await;
    let main = db.branch("main", "10000.00", "0").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (_, shift) = opened(&db, &main, &teller, &manager, thb_2300(&db).await).await;
    let over = vec![
        CountLine::new("THB", db.face("THB", "1000").await, 2),
        CountLine::new("THB", db.face("THB", "100").await, 4),
    ]; // 2,400: 100 over
    let counts = db.counts();
    let close = counts
        .submit(&teller, close_cmd(&main, &shift, over), at(600))
        .await
        .expect("a CLOSE count");
    counts
        .verify(&manager, &close.id, at(660))
        .await
        .expect("verified");
    assert_eq!(
        db.variance(&main, "THB").await,
        d("-100"),
        "THB 100 over came out of the branch's VARIANCE"
    );
    assert_eq!(db.at(CashPlace::Drawer(shift), "THB").await, d("2400.00"));
}
