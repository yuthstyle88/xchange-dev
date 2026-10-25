//! Up-front tests for #5106 (BE-1): certifying a shift CLOSE count returns the counted cash from the shift's
//! DRAWER to the branch's SAFE (owner's ruling 2026-10-25: "รับรองปิดกะ = ย้ายเข้าเซฟอัตโนมัติ (แนะนำ)").
//!
//! Written before the implementation by the test writer gemini/studio-10 (ticket author claude/bc8fd29a);
//! do not edit, delete or weaken.
//!
//! The defect at origin/main: `CashCountService::verify_in`, `CountKind::Close` branch
//! (`src/application/count/count_service.rs:1136-1320`), posts the count difference to `VARIANCE` and closes the
//! shift, moving nothing out of `DRAWER(shift)`. An `OPEN` verify draws from `SAFE(branch)`, so the drawer↔safe
//! cycle is one-way: closed drawers keep their balances and a subsequent shift's `OPEN` count with foreign currency
//! is refused because the safe never received it back.
//!
//! What this file pins ([required], §4 table):
//! - S1: exact `CLOSE` certified (THB 2,300) → `DRAWER(shift)` 0; `SAFE` THB += 2,300 (`close-to-safe:THB`).
//! - S2: short `CLOSE` within the limit (2,200 vs 2,300) → `VARIANCE` +100 as today; `SAFE` += 2,200; `DRAWER` 0.
//! - S3: `CLOSE` with THB and USD → `SAFE` gains both; `DRAWER` 0 in both (`close-to-safe:THB`, `close-to-safe:USD`).
//! - S4: `CLOSE` over the limit (`AwaitingFinance`) → nothing moves to the `SAFE`; after FINANCE approves and the
//!   count is verified again, S1/S2 hold.
//! - S5: after S3, a new shift's `OPEN` count with USD → verifies (the safe now holds USD).
//! - S6: a handover (`HANDOVER_OUT`) → unchanged: no `SAFE` posting.
//!
//! Modelled on `handover_variance_test.rs`. Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`;
//! nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::application::ledger_approval::count_difference::register as register_count_difference;
use xc_cash::domain::count::{CashCount, CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_cash::domain::handover::HandoverStatus;
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
    Utc.with_ymd_and_hms(2026, 10, 25, 2, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
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
        let schema = unique("clsafe");
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

    /// A place's USD balance in the ledger, through the port.
    async fn usd_at(&self, place: CashPlace) -> Decimal {
        let mut tx = self.pool.begin().await.unwrap();
        let b = LedgerCashPostings::new()
            .expected_in(&mut tx, &place)
            .await
            .expect("expected_in");
        tx.rollback().await.unwrap();
        b.get(usd()).value().normalize()
    }

    /// Posts an exchange buying USD into `shift`'s drawer (`Customer → Drawer(shift)` USD, `Bank → Customer` THB).
    async fn buy_usd_into_drawer(&self, shift: &str, amount_usd: &str) {
        let mut tx = self.pool.begin().await.unwrap();
        let source = SourceRef {
            source_type: SourceType::Exchange,
            source_id: unique("buy-usd"),
            source_key: "complete".to_string(),
        };
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[
                    PostingRequest {
                        from: CashPlace::Customer,
                        to: CashPlace::Drawer(shift.to_owned()),
                        currency: usd(),
                        quantity: ForeignAmount::from_decimal(d(amount_usd)),
                        source: source.clone(),
                        line: None,
                        reverses: None,
                    },
                    PostingRequest {
                        from: CashPlace::Bank,
                        to: CashPlace::Customer,
                        currency: CurrencyCode::THB,
                        quantity: ForeignAmount::from_decimal(d(amount_usd) * d("32.10")),
                        source,
                        line: None,
                        reverses: None,
                    },
                ],
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("buy USD into drawer: {e:?}"));
        tx.commit().await.unwrap();
    }

    async fn dec(&self, sql: &str) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .unwrap_or_default()
            .normalize()
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

    /// `source_key`s of `COUNT` postings for `count_id` with `close-to-safe:<currency>`, sorted.
    async fn close_to_safe_keys(&self, count_id: &str) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT source_key FROM ledger_postings \
             WHERE source_type = 'COUNT' AND source_id = $1 AND source_key LIKE 'close-to-safe:%' \
             ORDER BY source_key",
        )
        .bind(count_id)
        .fetch_all(&self.pool)
        .await
        .expect("read close-to-safe postings")
    }
}

/// THB 1000 × 2 + 100 × `hundreds`.
async fn thb(db: &Db, hundreds: i32) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.face("THB", "1000").await, 2),
        CountLine::new("THB", db.face("THB", "100").await, hundreds),
    ]
}

/// The floor: a branch with the given limit, teller A on a shift whose drawer holds THB 2,300 (SAFE holds THB 7,700
/// and USD 0), teller B to open or receive a subsequent shift, and a branch manager.
struct Floor {
    db: Db,
    main: BranchId,
    a: Actor,
    b: Actor,
    manager: Actor,
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
    assert_eq!(
        db.thb_at(CashPlace::Safe(main.clone())).await,
        d("7700"),
        "the fixture: SAFE holds THB 7,700 after A's OPEN count"
    );
    Floor {
        db,
        main,
        a,
        b,
        manager,
        a_shift,
    }
}

async fn submit_close(f: &Floor, lines: Vec<CountLine>) -> CashCount {
    CashCountService::pg(f.db.pool.clone())
        .submit(
            &f.a,
            SubmitCount {
                branch: f.main.clone(),
                shift_id: Some(f.a_shift.clone()),
                kind: CountKind::Close,
                lines,
                note: None,
                idempotency_key: unique("close"),
            },
            at(600),
        )
        .await
        .expect("A submits the CLOSE count")
}

// ── S1: exact CLOSE certified (THB 2,300) ────────────────────────────────────

#[tokio::test]
async fn s1_exact_close_certified_moves_counted_thb_from_drawer_to_safe() {
    let f = floor("500").await;
    let close = submit_close(&f, thb(&f.db, 3).await).await;
    CashCountService::pg(f.db.pool.clone())
        .verify(&f.manager, &close.id, at(660))
        .await
        .expect("manager certifies the exact CLOSE count");

    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("0"),
        "S1: DRAWER(shift) ends at 0 after CLOSE is certified"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Safe(f.main.clone())).await,
        d("10000"),
        "S1: SAFE THB gains the counted 2,300 (7,700 + 2,300 = 10,000)"
    );
    assert_eq!(
        f.db.close_to_safe_keys(&close.id).await,
        vec!["close-to-safe:THB".to_string()],
        "S1: posted under SourceType::Count, source_id = count.id, source_key = close-to-safe:THB"
    );
}

// ── S2: short CLOSE within the limit (2,200 vs 2,300) ────────────────────────

#[tokio::test]
async fn s2_short_close_within_limit_posts_variance_and_returns_counted_to_safe() {
    let f = floor("500").await;
    let close = submit_close(&f, thb(&f.db, 2).await).await; // THB 2,200 vs 2,300: 100 short
    CashCountService::pg(f.db.pool.clone())
        .verify(&f.manager, &close.id, at(660))
        .await
        .expect("manager certifies the short CLOSE count within the limit");

    assert_eq!(
        f.db.variance(&f.main).await,
        d("100"),
        "S2: VARIANCE +100 as today"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Safe(f.main.clone())).await,
        d("9900"),
        "S2: SAFE THB += 2,200 (7,700 + 2,200 = 9,900)"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("0"),
        "S2: DRAWER(shift) ends at 0 after VARIANCE and DRAWER→SAFE postings"
    );
    assert_eq!(
        f.db.close_to_safe_keys(&close.id).await,
        vec!["close-to-safe:THB".to_string()]
    );
}

// ── S3: CLOSE with THB and USD ───────────────────────────────────────────────

#[tokio::test]
async fn s3_close_with_thb_and_usd_moves_both_to_safe_and_empties_drawer() {
    let f = floor("500").await;
    f.db.buy_usd_into_drawer(&f.a_shift, "100.00").await;
    assert_eq!(
        f.db.usd_at(CashPlace::Safe(f.main.clone())).await,
        d("0"),
        "before CLOSE: SAFE holds no USD"
    );

    let mut lines = thb(&f.db, 3).await;
    lines.push(CountLine::new("USD", f.db.face("USD", "100").await, 1));
    let close = submit_close(&f, lines).await;
    CashCountService::pg(f.db.pool.clone())
        .verify(&f.manager, &close.id, at(660))
        .await
        .expect("manager certifies the two-currency CLOSE count");

    assert_eq!(
        f.db.thb_at(CashPlace::Safe(f.main.clone())).await,
        d("10000"),
        "S3: SAFE gains THB 2,300"
    );
    assert_eq!(
        f.db.usd_at(CashPlace::Safe(f.main.clone())).await,
        d("100"),
        "S3: SAFE gains USD 100"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("0"),
        "S3: DRAWER 0 in THB"
    );
    assert_eq!(
        f.db.usd_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("0"),
        "S3: DRAWER 0 in USD"
    );
    assert_eq!(
        f.db.close_to_safe_keys(&close.id).await,
        vec![
            "close-to-safe:THB".to_string(),
            "close-to-safe:USD".to_string(),
        ],
        "S3: close-to-safe:<currency> posted for both currencies"
    );
}

// ── S4: CLOSE over the limit (AwaitingFinance), then approved and verified ───

#[tokio::test]
async fn s4_close_over_limit_moves_nothing_until_finance_approves_and_count_is_reverified() {
    let f = floor("50").await;
    let finance = f.db.person("Somchai", &[Role::Finance]).await;
    let counts = CashCountService::pg(f.db.pool.clone());
    let close = submit_close(&f, thb(&f.db, 2).await).await; // THB 2,200 vs 2,300: 100 short, over 50 limit

    let req_id = match counts.verify(&f.manager, &close.id, at(660)).await {
        Err(AppError::Domain(msg)) => {
            let id: String = sqlx::query_scalar(
                "SELECT id::text FROM approval_requests WHERE type = 'COUNT_DIFFERENCE' AND object_id = $1",
            )
            .bind(&close.id)
            .fetch_one(&f.db.pool)
            .await
            .expect("the PENDING COUNT_DIFFERENCE request");
            assert_eq!(msg, CashError::AwaitingFinance(id.clone()).to_string());
            id
        }
        other => panic!("expected AwaitingFinance refusal, got {other:?}"),
    };

    // While AwaitingFinance: nothing moves to the SAFE, drawer stays at 2,300, VARIANCE is 0.
    assert_eq!(
        f.db.thb_at(CashPlace::Safe(f.main.clone())).await,
        d("7700"),
        "S4 (AwaitingFinance): nothing moves to the SAFE"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("2300"),
        "S4 (AwaitingFinance): DRAWER untouched"
    );
    assert_eq!(f.db.variance(&f.main).await, d("0"));
    assert!(f.db.close_to_safe_keys(&close.id).await.is_empty());

    // FINANCE approves the COUNT_DIFFERENCE request, and the manager verifies the CLOSE count again.
    let mut registry = ApprovalRegistry::new();
    register_count_difference(&mut registry, f.db.pool.clone());
    let approvals = ApprovalService::new(
        registry,
        Arc::new(PgApprovalRepository::new(f.db.pool.clone())),
        Arc::new(PgAuditLog::new(f.db.pool.clone())),
    );
    let mut tx = f.db.pool.begin().await.unwrap();
    approvals
        .decide(
            &mut tx,
            &RequestId::new(&req_id),
            &finance,
            Outcome::Approved,
            "approved difference",
            &unique("decide"),
        )
        .await
        .expect("FINANCE approves the count difference");
    tx.commit().await.unwrap();

    counts
        .verify(&f.manager, &close.id, at(700))
        .await
        .expect("CLOSE count verifies after FINANCE approves");

    assert_eq!(
        f.db.variance(&f.main).await,
        d("100"),
        "S4 (after approval): VARIANCE +100"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Safe(f.main.clone())).await,
        d("9900"),
        "S4 (after approval): SAFE THB += 2,200 (7,700 + 2,200 = 9,900)"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("0"),
        "S4 (after approval): DRAWER 0"
    );
    assert_eq!(
        f.db.close_to_safe_keys(&close.id).await,
        vec!["close-to-safe:THB".to_string()]
    );
}

// ── S5: after S3, a new shift's OPEN count with USD verifies ─────────────────

#[tokio::test]
async fn s5_after_s3_a_new_shifts_open_count_with_usd_verifies() {
    let f = floor("500").await;
    f.db.buy_usd_into_drawer(&f.a_shift, "100.00").await;

    let mut close_lines = thb(&f.db, 3).await;
    close_lines.push(CountLine::new("USD", f.db.face("USD", "100").await, 1));
    let close = submit_close(&f, close_lines).await;
    let counts = CashCountService::pg(f.db.pool.clone());
    counts
        .verify(&f.manager, &close.id, at(660))
        .await
        .expect("A's CLOSE count with THB and USD is certified");

    let mut open_lines = thb(&f.db, 3).await;
    open_lines.push(CountLine::new("USD", f.db.face("USD", "100").await, 1));
    let open_b = counts
        .submit(
            &f.b,
            SubmitCount {
                branch: f.main.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: open_lines,
                note: None,
                idempotency_key: unique("open-b"),
            },
            at(720),
        )
        .await
        .expect("B submits an OPEN count with THB and USD");
    let b_shift = counts
        .verify(&f.manager, &open_b.id, at(780))
        .await
        .expect(
            "S5: B's OPEN count with USD verifies because A's close returned USD 100 to the SAFE",
        )
        .shift_id
        .expect("B's shift");

    assert_eq!(
        f.db.usd_at(CashPlace::Drawer(b_shift)).await,
        d("100"),
        "S5: B's new drawer holds USD 100 drawn from the SAFE"
    );
    assert_eq!(
        f.db.usd_at(CashPlace::Safe(f.main)).await,
        d("0"),
        "S5: the SAFE's USD 100 moved into B's drawer"
    );
}

// ── S6: a handover (HANDOVER_OUT) is unchanged (no SAFE posting) ─────────────

#[tokio::test]
async fn s6_a_handover_is_unchanged_and_posts_nothing_to_the_safe() {
    let f = floor("500").await;
    let h = HandoverService::pg(f.db.pool.clone())
        .start(
            &f.a,
            StartHandover {
                branch: f.main.clone(),
                from_shift_id: f.a_shift.clone(),
                receiver_id: f.b.user_id.clone(),
                lines: thb(&f.db, 3).await,
                note: None,
                idempotency_key: unique("handover"),
            },
            at(600),
        )
        .await
        .expect("A starts the handover");
    let accepted = HandoverService::pg(f.db.pool.clone())
        .accept(&f.b, &h.id, at(700))
        .await
        .expect("B accepts the handover");
    assert_eq!(accepted.status, HandoverStatus::Accepted);
    let b_shift = accepted.to_shift_id.expect("B's shift");

    assert_eq!(
        f.db.thb_at(CashPlace::Safe(f.main)).await,
        d("7700"),
        "S6: SAFE THB stays at 7,700 (handover moves DRAWER(A) → DRAWER(B), not to SAFE)"
    );
    assert_eq!(f.db.thb_at(CashPlace::Drawer(f.a_shift)).await, d("0"));
    assert_eq!(f.db.thb_at(CashPlace::Drawer(b_shift)).await, d("2300"));
    assert!(
        f.db.close_to_safe_keys(&h.count_id).await.is_empty(),
        "S6: no close-to-safe posting on a handover"
    );
}
