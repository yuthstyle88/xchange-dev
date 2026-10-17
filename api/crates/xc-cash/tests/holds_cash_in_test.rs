//! Q3 (#2957 / #2961, part of #626): `HandoverService::holds_cash_in` answers whether an employee
//! holds an OPEN shift of their own, or is the sender or the receiver of a PENDING hand-over in
//! that organization.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const OTHER_ORG: &str = "00000000-0000-0000-0000-000000000002";

static NEXT: AtomicU64 = AtomicU64::new(0);
static LG4_FUND: AtomicU64 = AtomicU64::new(0);

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

struct Db {
    pool: PgPool,
    admin: PgPool,
    schema: String,
}

impl Db {
    async fn fresh() -> Db {
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
        let schema = unique("ca10_holds_cash");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        migrator
            .run(&pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        Db {
            pool,
            admin,
            schema,
        }
    }

    async fn close(self) {
        self.pool.close().await;
        let _ = sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await;
        self.admin.close().await;
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

    async fn person_with_legacy(
        &self,
        name: &str,
        legacy_employee_id: Option<&str>,
        roles: &[Role],
    ) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test', $3) RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .bind(legacy_employee_id)
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

    async fn person(&self, name: &str, roles: &[Role]) -> Actor {
        self.person_with_legacy(name, None, roles).await
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .expect("denomination")
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    fn handovers(&self) -> HandoverService {
        HandoverService::pg(self.pool.clone())
    }
}

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
                        LG4_FUND.fetch_add(1, Ordering::Relaxed)
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

async fn thb_2300(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
    ]
}

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
        .expect("teller submits OPEN count");
    counts
        .verify(manager, &count.id, at(60))
        .await
        .expect("manager verifies OPEN count")
        .shift_id
        .expect("shift id")
}

// ── 1. open shift → true ────────────────────────────────────────────────────

#[tokio::test]
async fn open_shift_returns_true() {
    let db = Db::fresh().await;
    let branch = db.branch("b1").await;
    let legacy_id = unique("emp-open");
    let teller = db
        .person_with_legacy("Ploy", Some(&legacy_id), &[Role::Teller])
        .await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let _shift_id = opened(&db, &branch, &teller, &manager).await;

    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        HandoverService::holds_cash_in(&mut tx, ORG, &teller.user_id)
            .await
            .unwrap(),
        "teller with an OPEN shift holds cash (by users.id)"
    );
    assert!(
        HandoverService::holds_cash_in(&mut tx, ORG, &legacy_id)
            .await
            .unwrap(),
        "teller with an OPEN shift holds cash (by legacy_employee_id)"
    );
    tx.commit().await.unwrap();
    db.close().await;
}

// ── 2. sender of a PENDING hand-over → true ─────────────────────────────────

#[tokio::test]
async fn sender_of_pending_handover_returns_true() {
    let db = Db::fresh().await;
    let branch = db.branch("b2").await;
    let sender = db.person("Ploy", &[Role::Teller]).await;
    let receiver = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let from_shift = opened(&db, &branch, &sender, &manager).await;
    db.handovers()
        .start(
            &sender,
            StartHandover {
                branch: branch.clone(),
                from_shift_id: from_shift.clone(),
                receiver_id: receiver.user_id.clone(),
                lines: thb_2300(&db).await,
                note: None,
                idempotency_key: unique("ho-sender"),
            },
            at(120),
        )
        .await
        .expect("start handover");

    // Isolate the PENDING hand-over sender check from the OPEN-shift check so
    // dropping the sender half in holds_cash_in makes this test go red.
    sqlx::query("UPDATE shifts SET status = 'CLOSED' WHERE id = $1")
        .bind(&from_shift)
        .execute(&db.pool)
        .await
        .unwrap();

    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        HandoverService::holds_cash_in(&mut tx, ORG, &sender.user_id)
            .await
            .unwrap(),
        "sender of a PENDING hand-over holds cash"
    );
    tx.commit().await.unwrap();
    db.close().await;
}

// ── 3. receiver of a PENDING hand-over → true ───────────────────────────────

#[tokio::test]
async fn receiver_of_pending_handover_returns_true() {
    let db = Db::fresh().await;
    let branch = db.branch("b3").await;
    let sender = db.person("Ploy", &[Role::Teller]).await;
    let receiver = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let from_shift = opened(&db, &branch, &sender, &manager).await;
    db.handovers()
        .start(
            &sender,
            StartHandover {
                branch: branch.clone(),
                from_shift_id: from_shift,
                receiver_id: receiver.user_id.clone(),
                lines: thb_2300(&db).await,
                note: None,
                idempotency_key: unique("ho-receiver"),
            },
            at(120),
        )
        .await
        .expect("start handover");

    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        HandoverService::holds_cash_in(&mut tx, ORG, &receiver.user_id)
            .await
            .unwrap(),
        "receiver of a PENDING hand-over holds cash"
    );
    tx.commit().await.unwrap();
    db.close().await;
}

// ── 4. a decided hand-over only → false ─────────────────────────────────────

#[tokio::test]
async fn decided_handover_only_returns_false() {
    let db = Db::fresh().await;
    let branch = db.branch("b4").await;
    let sender = db.person("Ploy", &[Role::Teller]).await;
    let receiver = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let from_shift = opened(&db, &branch, &sender, &manager).await;
    let rejected = db
        .handovers()
        .start(
            &sender,
            StartHandover {
                branch: branch.clone(),
                from_shift_id: from_shift.clone(),
                receiver_id: receiver.user_id.clone(),
                lines: thb_2300(&db).await,
                note: None,
                idempotency_key: unique("ho-reject"),
            },
            at(120),
        )
        .await
        .expect("start handover");
    db.handovers()
        .reject(&receiver, &rejected.id, at(180))
        .await
        .expect("reject handover");

    let accepted = db
        .handovers()
        .start(
            &sender,
            StartHandover {
                branch: branch.clone(),
                from_shift_id: from_shift,
                receiver_id: receiver.user_id.clone(),
                lines: thb_2300(&db).await,
                note: None,
                idempotency_key: unique("ho-accept"),
            },
            at(240),
        )
        .await
        .expect("start second handover");
    let accepted = db
        .handovers()
        .accept(&receiver, &accepted.id, at(300))
        .await
        .expect("accept handover");

    // Close receiver's opened shift so neither teller holds an OPEN shift; only
    // decided (REJECTED + ACCEPTED) hand-overs remain.
    sqlx::query("UPDATE shifts SET status = 'CLOSED' WHERE id = $1")
        .bind(accepted.to_shift_id.as_deref().expect("to_shift_id"))
        .execute(&db.pool)
        .await
        .unwrap();

    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        !HandoverService::holds_cash_in(&mut tx, ORG, &sender.user_id)
            .await
            .unwrap(),
        "sender of decided hand-overs only does not hold cash"
    );
    assert!(
        !HandoverService::holds_cash_in(&mut tx, ORG, &receiver.user_id)
            .await
            .unwrap(),
        "receiver of decided hand-overs only does not hold cash"
    );
    tx.commit().await.unwrap();
    db.close().await;
}

// ── 5. none → false ─────────────────────────────────────────────────────────

#[tokio::test]
async fn none_returns_false() {
    let db = Db::fresh().await;
    let _branch = db.branch("b5").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;

    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        !HandoverService::holds_cash_in(&mut tx, ORG, &teller.user_id)
            .await
            .unwrap(),
        "employee with no shift and no hand-over does not hold cash"
    );
    assert!(
        !HandoverService::holds_cash_in(&mut tx, ORG, "no-such-employee")
            .await
            .unwrap(),
        "unknown employee does not hold cash"
    );
    tx.commit().await.unwrap();
    db.close().await;
}

// ── 6. another organization's hand-over → false ─────────────────────────────

#[tokio::test]
async fn another_organizations_handover_returns_false() {
    let db = Db::fresh().await;
    let sender = db.person("Ploy", &[Role::Teller]).await;
    let receiver = db.person("Nok", &[Role::Teller]).await;

    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1::uuid, 'OTHER', NULL, 'Asia/Bangkok', now()) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(OTHER_ORG)
    .execute(&db.pool)
    .await
    .unwrap();

    let other_branch = unique("other_branch");
    sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
        .bind(&other_branch)
        .bind(OTHER_ORG)
        .execute(&db.pool)
        .await
        .unwrap();

    let other_shift = unique("other_shift");
    sqlx::query(
        "INSERT INTO shifts (id, organization_id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
         VALUES ($1, $2::uuid, $3, 'CLOSED', now(), 'Ploy', '', $4, $5::uuid)",
    )
    .bind(&other_shift)
    .bind(OTHER_ORG)
    .bind(&other_branch)
    .bind(unique("req"))
    .bind(&sender.user_id)
    .execute(&db.pool)
    .await
    .unwrap();

    let other_count = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO cash_counts (id, organization_id, branch_id, shift_id, kind, counted_by, counted_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, $4, 'HANDOVER_OUT', $5::uuid, now(), $6)",
    )
    .bind(other_count)
    .bind(OTHER_ORG)
    .bind(&other_branch)
    .bind(&other_shift)
    .bind(&sender.user_id)
    .bind(unique("idem"))
    .execute(&db.pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO handovers (organization_id, branch_id, from_shift_id, count_id, receiver_id, status) \
         VALUES ($1::uuid, $2, $3, $4, $5::uuid, 'PENDING')",
    )
    .bind(OTHER_ORG)
    .bind(&other_branch)
    .bind(&other_shift)
    .bind(other_count)
    .bind(&receiver.user_id)
    .execute(&db.pool)
    .await
    .unwrap();

    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        !HandoverService::holds_cash_in(&mut tx, ORG, &sender.user_id)
            .await
            .unwrap(),
        "another organization's PENDING hand-over does not count for sender in ORG"
    );
    assert!(
        !HandoverService::holds_cash_in(&mut tx, ORG, &receiver.user_id)
            .await
            .unwrap(),
        "another organization's PENDING hand-over does not count for receiver in ORG"
    );
    tx.commit().await.unwrap();
    db.close().await;
}
