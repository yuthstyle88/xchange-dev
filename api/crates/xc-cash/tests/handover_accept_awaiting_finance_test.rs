//! Up-front tests (P7): a hand-over accept that waits for FINANCE answers it as a value on the caller's transaction,
//! so the `COUNT_DIFFERENCE` request commits with it, the way `CashCountService::verify_in` answers
//! `CountVerified::AwaitingFinance` (design §4.9, `api/src/presentation/http/v3/commands/cash.rs` `CountVerify`).
//!
//! Written before the implementation by the ticket author (claude/bc8fd29a, xchange BE lane controller).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The defect at origin/main `b30ec019b`: `HandoverService::accept_in` answers awaiting FINANCE as
//! `Err(CashError::AwaitingFinance(id))`. The v3 `handoverAccept` command (`v3/commands/cash/handovers.rs`) maps that
//! to an error, so the command's transaction rolls back and the request `CountDifferenceGate::check_in` inserted is
//! gone: FINANCE never sees it, every retry mints a new id, and a hand-over with a difference over the limit can
//! never be accepted (xc-v3 build `bf306f5c`: requests `e6608f49…` and `8bae3eb8…` do not exist).
//!
//! The new seam is `HandoverService::accept_or_await_in(tx, actor, id, now) -> AppResult<HandoverAccepted>`, with
//! `HandoverAccepted::{Accepted(Box<Handover>), AwaitingFinance(String)}` exported from
//! `xc_cash::application::handover`, shaped like `CountVerified`. `accept_in` keeps its signature and its
//! `Err(AwaitingFinance)` answer (callers in `services_on_caller_tx_test.rs` and `handover_unknown_id_test.rs`), and
//! `accept` keeps committing then answering `Err(AwaitingFinance)` (`handover_variance_test.rs` H4).
//!
//! What this file pins ([required]):
//! - P7-1: over the limit, `accept_or_await_in` answers `Ok(HandoverAccepted::AwaitingFinance(<request id>))`; once
//!   the caller commits, exactly one PENDING `COUNT_DIFFERENCE` request with that id exists for the hand-over's count,
//!   and nothing moved (hand-over PENDING, giver's shift OPEN with THB 2,300, VARIANCE untouched).
//! - P7-2: asking again answers the same request id and adds no request.
//! - P7-3: once that request is APPROVED, it answers `Ok(HandoverAccepted::Accepted(_))` with status ACCEPTED, and
//!   the giver's drawer is emptied.
//! - P7-4: within the limit, it answers `Ok(HandoverAccepted::Accepted(_))` and raises no request.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverAccepted, HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
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
        let schema = unique("hoaf");
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

/// B accepts on a caller transaction, which is then committed whatever the answer.
async fn accept_committed(f: &Floor, h: &Handover, seconds: i64) -> HandoverAccepted {
    let mut tx = f.db.pool.begin().await.expect("begin");
    let answer = HandoverService::pg(f.db.pool.clone())
        .accept_or_await_in(&mut tx, &f.b, &h.id, at(seconds))
        .await
        .unwrap_or_else(|e| {
            panic!("accept_or_await_in answers awaiting FINANCE as a value, not an error: {e:?}")
        });
    tx.commit().await.expect("the caller commits");
    answer
}

// ── P7-1: over the limit, the request commits with the caller ────────────────

#[tokio::test]
async fn p7_1_accept_in_over_the_limit_answers_awaiting_finance_and_its_request_commits() {
    let f = floor("50").await;
    let h = start(&f, 2).await; // THB 2,200 against 2,300: 100 short, over the 50 limit

    let req_id = match accept_committed(&f, &h, 700).await {
        HandoverAccepted::AwaitingFinance(id) => id,
        other => panic!("over the limit must answer AwaitingFinance; got {other:?}"),
    };

    let requests = f.db.count_difference_requests(&h.count_id).await;
    assert_eq!(
        requests.len(),
        1,
        "exactly one COUNT_DIFFERENCE request survives the commit: {requests:?}"
    );
    assert_eq!(requests[0].0, req_id, "the answer names the stored request");
    assert_eq!(requests[0].1, "PENDING");
    assert_eq!(f.db.handover_status(&h.id).await, "PENDING");
    assert_eq!(f.db.shift_status(&f.a_shift).await, "OPEN");
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

// ── P7-2: asking again names the same request ────────────────────────────────

#[tokio::test]
async fn p7_2_asking_again_answers_the_same_request_and_adds_none() {
    let f = floor("50").await;
    let h = start(&f, 2).await;

    let first = match accept_committed(&f, &h, 700).await {
        HandoverAccepted::AwaitingFinance(id) => id,
        other => panic!("first ask must answer AwaitingFinance; got {other:?}"),
    };
    let second = match accept_committed(&f, &h, 760).await {
        HandoverAccepted::AwaitingFinance(id) => id,
        other => panic!("second ask must answer AwaitingFinance; got {other:?}"),
    };
    assert_eq!(first, second, "the retry names the request already waiting");
    assert_eq!(f.db.count_difference_requests(&h.count_id).await.len(), 1);
}

// ── P7-3: once FINANCE approves, the accept goes through ─────────────────────

#[tokio::test]
async fn p7_3_once_the_request_is_approved_accept_in_accepts() {
    let f = floor("50").await;
    let h = start(&f, 2).await;

    let req_id = match accept_committed(&f, &h, 700).await {
        HandoverAccepted::AwaitingFinance(id) => id,
        other => panic!("over the limit must answer AwaitingFinance; got {other:?}"),
    };
    sqlx::query("UPDATE approval_requests SET status = 'APPROVED' WHERE id = $1::uuid")
        .bind(&req_id)
        .execute(&f.db.pool)
        .await
        .expect("FINANCE approves the request");

    match accept_committed(&f, &h, 800).await {
        HandoverAccepted::Accepted(accepted) => {
            assert_eq!(accepted.status, HandoverStatus::Accepted);
            assert!(accepted.to_shift_id.is_some(), "B gets a shift");
        }
        other => panic!("an approved difference must be accepted; got {other:?}"),
    }
    assert_eq!(f.db.handover_status(&h.id).await, "ACCEPTED");
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("0"),
        "A's drawer is emptied"
    );
}

// ── P7-4: within the limit nothing waits ─────────────────────────────────────

#[tokio::test]
async fn p7_4_within_the_limit_accept_in_accepts_and_raises_no_request() {
    let f = floor("500").await;
    let h = start(&f, 2).await; // 100 short, within the 500 limit

    match accept_committed(&f, &h, 700).await {
        HandoverAccepted::Accepted(accepted) => {
            assert_eq!(accepted.status, HandoverStatus::Accepted)
        }
        other => panic!("within the limit must be accepted; got {other:?}"),
    }
    assert!(f.db.count_difference_requests(&h.count_id).await.is_empty());
}
