//! Up-front tests: a hand-over whose count differs from the drawer passes the same FINANCE gate as a close count
//! and posts the difference to the branch's VARIANCE (owner's ruling 2026-10-24).
//!
//! Written before the implementation by the ticket author's helper (for claude/7ca80c, xchange BE lane controller).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The defect at origin/main `3bac87e0a`: `HandoverService::accept_in`
//! (`src/application/handover/handover_service.rs`) writes the cash exception and posts only
//! `DRAWER(from) → DRAWER(to)` of what was counted. It never runs `CountDifferenceGate::check_in` and never posts
//! `DRAWER ↔ VARIANCE`, which the close-count verify does (`src/application/count/count_service.rs`, the
//! `CountKind::Close` branch of `verify_in`). So a short hand-over leaves the shortfall in the giver's closed drawer
//! forever, and an over hand-over can never be accepted (`stockInsufficient`: it moves more than the drawer holds).
//!
//! What this file pins ([required]):
//! - H1 short: the difference goes `DRAWER(from) → VARIANCE(branch)`, the giver's drawer ends at 0, the receiver's
//!   drawer opens at what was counted; within the branch's limit no FINANCE request is raised.
//! - H2 over: the difference comes `VARIANCE(branch) → DRAWER(from)`, then everything counted moves to the receiver;
//!   the accept succeeds within the limit.
//! - H3 exact (guard): no VARIANCE posting, no FINANCE request.
//! - H4 over the limit: the accept is refused exactly as a close-count verify is
//!   (`count_verify_gate_test.rs::verify_count_difference_exceeding_branch_limit_refuses_awaiting_finance`):
//!   `AppError::Domain(CashError::AwaitingFinance(<request id>))`, one PENDING `COUNT_DIFFERENCE` request for the
//!   hand-over's count that survives the refusal, and nothing moves: the hand-over stays PENDING, the giver's shift
//!   stays OPEN with its drawer untouched, the receiver has no shift, VARIANCE is untouched.
//!
//! Balances are read from the ledger (`LedgerCashPostings::expected_in`), VARIANCE with SQL, the way
//! `lg4_cutover_counts_test.rs` reads them. Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`;
//! nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;

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
use xc_cash::domain::handover::{Handover, HandoverStatus};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 24, 2, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
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
        let schema = unique("hovar");
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

    /// A branch with THB 10,000 in its SAFE (ledger) and a count-difference limit of `limit_thb`.
    async fn branch(&self, limit_thb: &str) -> BranchId {
        let id = unique("main");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, $3::numeric, 'test', now())",
        )
        .bind(ORG)
        .bind(&id)
        .bind(limit_thb)
        .execute(&self.pool)
        .await
        .expect("a count-difference limit");
        let b = BranchId::new(id);
        let mut tx = self.pool.begin().await.unwrap();
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Safe(b.clone()),
                    currency: CurrencyCode::THB,
                    quantity: ForeignAmount::from_decimal(d("10000.00")),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: unique("fund"),
                        source_key: "thb".to_string(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("fund the safe: {e:?}"));
        tx.commit().await.unwrap();
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
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
                 VALUES ($1::uuid, $2, $3::uuid, NULL, '2026-01-01T00:00:00Z')",
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

    /// A place's THB balance in the ledger, through the port.
    async fn thb_at(&self, place: CashPlace) -> Decimal {
        let mut tx = self.pool.begin().await.unwrap();
        let b = LedgerCashPostings::new()
            .expected_in(&mut tx, &place)
            .await
            .expect("expected_in");
        tx.rollback().await.unwrap();
        b.get(CurrencyCode::THB).value().normalize()
    }

    async fn dec(&self, sql: &str) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .unwrap_or_default()
            .normalize()
    }

    async fn int(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// The THB `VARIANCE` balance of `branch` (positive = cash the drawers were short of).
    async fn variance(&self, branch: &BranchId) -> Decimal {
        self.dec(&format!(
            "SELECT sum(e.quantity_signed) FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
             WHERE a.kind = 'VARIANCE' AND a.branch_id = '{}' AND trim(a.currency) = 'THB'",
            branch.as_str()
        ))
        .await
    }

    /// How many ledger entries touch `branch`'s VARIANCE at all.
    async fn variance_entries(&self, branch: &BranchId) -> i64 {
        self.int(&format!(
            "SELECT count(*) FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
             WHERE a.kind = 'VARIANCE' AND a.branch_id = '{}'",
            branch.as_str()
        ))
        .await
    }

    /// `(id, status, proposed_payload)` of every `COUNT_DIFFERENCE` request for `count_id`.
    async fn count_difference_requests(
        &self,
        count_id: &str,
    ) -> Vec<(String, String, serde_json::Value)> {
        sqlx::query_as(
            "SELECT id::text, status, proposed_payload FROM approval_requests \
             WHERE type = 'COUNT_DIFFERENCE' AND object_id = $1",
        )
        .bind(count_id)
        .fetch_all(&self.pool)
        .await
        .expect("read approval requests")
    }

    async fn open_shift_of(&self, teller: &Actor) -> Option<String> {
        sqlx::query_scalar("SELECT id FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'")
            .bind(&teller.user_id)
            .fetch_optional(&self.pool)
            .await
            .expect("read the teller's open shift")
    }

    async fn shift_status(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM shifts WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read the shift status")
    }

    async fn handover_status(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM handovers WHERE id = $1::uuid")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read the hand-over status")
    }

    async fn count_verified_by(&self, id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT verified_by::text FROM cash_counts WHERE id = $1::uuid")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read the count's verified_by")
    }
}

/// THB 1000 × 2 + 100 × `hundreds`.
async fn thb(db: &Db, hundreds: i32) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.face("THB", "1000").await, 2),
        CountLine::new("THB", db.face("THB", "100").await, hundreds),
    ]
}

/// The floor: a branch with the given limit, teller A on a shift whose drawer holds THB 2,300, teller B to receive.
struct Floor {
    db: Db,
    main: BranchId,
    a: Actor,
    b: Actor,
    a_shift: String,
}

async fn floor(limit_thb: &str) -> Floor {
    let db = Db::fresh().await;
    let main = db.branch(limit_thb).await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let counts = CashCountService::pg(db.pool.clone());
    let open = counts
        .submit(
            &a,
            SubmitCount {
                branch: main.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: thb(&db, 3).await,
                note: None,
                idempotency_key: unique("open"),
            },
            at(0),
        )
        .await
        .expect("A submits the OPEN count");
    let a_shift = counts
        .verify(&manager, &open.id, at(60))
        .await
        .expect("the BRANCH_MANAGER verifies the OPEN count")
        .shift_id
        .expect("A's shift");
    assert_eq!(
        db.thb_at(CashPlace::Drawer(a_shift.clone())).await,
        d("2300"),
        "the fixture: A's drawer expects THB 2,300"
    );
    Floor {
        db,
        main,
        a,
        b,
        a_shift,
    }
}

/// A starts a hand-over to B counting THB 2,000 + 100 × `hundreds`.
async fn start(f: &Floor, hundreds: i32) -> Handover {
    HandoverService::pg(f.db.pool.clone())
        .start(
            &f.a,
            StartHandover {
                branch: f.main.clone(),
                from_shift_id: f.a_shift.clone(),
                receiver_id: f.b.user_id.clone(),
                lines: thb(&f.db, hundreds).await,
                note: None,
                idempotency_key: unique("handover"),
            },
            at(600),
        )
        .await
        .expect("A starts the hand-over")
}

// ── H1: a short hand-over within the limit ───────────────────────────────────

#[tokio::test]
async fn h1_a_short_handover_puts_the_shortfall_on_variance_and_empties_the_givers_drawer() {
    let f = floor("500").await;
    let h = start(&f, 2).await; // THB 2,200 against 2,300: 100 short, within the 500 limit
    let accepted = HandoverService::pg(f.db.pool.clone())
        .accept(&f.b, &h.id, at(700))
        .await
        .expect("B accepts a short hand-over within the branch's limit");
    assert_eq!(accepted.status, HandoverStatus::Accepted);
    let b_shift = accepted.to_shift_id.expect("B's shift");

    assert_eq!(
        f.db.variance(&f.main).await,
        d("100"),
        "the THB 100 shortfall is posted DRAWER(A) → VARIANCE(branch), as a short close count is"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("0"),
        "A's closed drawer is empty: nothing is stranded in it"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(b_shift)).await,
        d("2200"),
        "B's drawer opens at what the hand-over counted"
    );
    assert!(
        f.db.count_difference_requests(&h.count_id).await.is_empty(),
        "a difference within the limit raises no FINANCE request"
    );
}

// ── H2: an over hand-over within the limit ───────────────────────────────────

#[tokio::test]
async fn h2_an_over_handover_takes_the_overage_from_variance_and_is_accepted() {
    let f = floor("500").await;
    let h = start(&f, 4).await; // THB 2,400 against 2,300: 100 over, within the 500 limit
    let accepted = HandoverService::pg(f.db.pool.clone())
        .accept(&f.b, &h.id, at(700))
        .await
        .expect("B accepts an over hand-over within the branch's limit");
    assert_eq!(accepted.status, HandoverStatus::Accepted);
    let b_shift = accepted.to_shift_id.expect("B's shift");

    assert_eq!(
        f.db.variance(&f.main).await,
        d("-100"),
        "the THB 100 overage came VARIANCE(branch) → DRAWER(A), as an over close count does"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("0"),
        "A's closed drawer is empty"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(b_shift)).await,
        d("2400"),
        "B's drawer opens at what the hand-over counted"
    );
    assert!(
        f.db.count_difference_requests(&h.count_id).await.is_empty(),
        "a difference within the limit raises no FINANCE request"
    );
}

// ── H3 (guard): an exact hand-over ───────────────────────────────────────────

#[tokio::test]
async fn h3_an_exact_handover_posts_nothing_to_variance() {
    let f = floor("500").await;
    let h = start(&f, 3).await; // THB 2,300: exact
    let accepted = HandoverService::pg(f.db.pool.clone())
        .accept(&f.b, &h.id, at(700))
        .await
        .expect("B accepts an exact hand-over");
    let b_shift = accepted.to_shift_id.expect("B's shift");

    assert_eq!(
        f.db.variance_entries(&f.main).await,
        0,
        "no VARIANCE posting for an exact count"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("0")
    );
    assert_eq!(f.db.thb_at(CashPlace::Drawer(b_shift)).await, d("2300"));
    assert!(f.db.count_difference_requests(&h.count_id).await.is_empty());
}

// ── H4: a difference over the limit waits for FINANCE ────────────────────────

#[tokio::test]
async fn h4_a_handover_difference_over_the_limit_is_refused_awaiting_finance_and_moves_nothing() {
    let f = floor("50").await;
    let h = start(&f, 2).await; // THB 2,200 against 2,300: 100 short, over the 50 limit
    let res = HandoverService::pg(f.db.pool.clone())
        .accept(&f.b, &h.id, at(700))
        .await;

    match res {
        Err(AppError::Domain(msg)) => {
            let requests = f.db.count_difference_requests(&h.count_id).await;
            assert_eq!(
                requests.len(),
                1,
                "exactly one COUNT_DIFFERENCE request for the hand-over's count, kept after the refusal: \
                 {requests:?}"
            );
            let (req_id, status, payload) = &requests[0];
            assert_eq!(status, "PENDING");
            assert_eq!(payload["count_id"], h.count_id);
            assert_eq!(
                d(payload["net_difference_thb"]
                    .as_str()
                    .expect("net_difference_thb is a string")),
                d("-100"),
                "net_difference_thb is the hand-over's difference"
            );
            assert_eq!(
                msg,
                CashError::AwaitingFinance(req_id.clone()).to_string(),
                "the refusal reaches the caller as AppError::Domain of CashError::AwaitingFinance, as at close count"
            );
        }
        other => panic!(
            "a hand-over difference over the branch's limit must be refused AwaitingFinance, as a close count \
             is; got {other:?}"
        ),
    }

    assert_eq!(f.db.handover_status(&h.id).await, "PENDING");
    assert_eq!(f.db.shift_status(&f.a_shift).await, "OPEN");
    assert!(f.db.count_verified_by(&h.count_id).await.is_none());
    assert!(f.db.open_shift_of(&f.b).await.is_none(), "B gets no shift");
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("2300"),
        "A's drawer is untouched"
    );
    assert_eq!(
        f.db.variance_entries(&f.main).await,
        0,
        "VARIANCE untouched"
    );
}
