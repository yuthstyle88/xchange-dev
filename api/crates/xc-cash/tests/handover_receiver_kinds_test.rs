//! CA-4 / #4886: Mid-shift hand-over receivers (closing own shift first, branch manager, moved-in teller with a dated role).

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::handover_service::INVALID_RECEIVER;
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_cash::domain::handover::HandoverStatus;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const NOW: i64 = 600;
const DAY: i64 = 86_400;

static LG4_FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn refused_with<T: std::fmt::Debug>(result: Result<T, AppError>, expected: &str) {
    match result {
        Err(AppError::Domain(sentence)) => assert_eq!(sentence, expected),
        other => panic!("expected AppError::Domain({expected:?}), got {other:?}"),
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
        let schema = unique("hrk");
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
        let to_dec = |minor: i128| Decimal::from_i128_with_scale(minor, 2).normalize();
        (
            to_dec(row.opening.minor_i128()),
            to_dec(row.expected.minor()),
            row.counted.map(|m| to_dec(m.minor_i128())),
        )
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    fn handovers(&self) -> HandoverService {
        HandoverService::pg(self.pool.clone())
    }

    async fn handover_rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM handovers")
            .fetch_one(&self.pool)
            .await
            .expect("count handovers")
    }
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
        .expect("submit OPEN count");
    counts
        .verify(manager, &count.id, at(60))
        .await
        .expect("verify OPEN count")
        .shift_id
        .expect("shift id")
}

async fn receiver(
    db: &Db,
    role: Role,
    branch: Option<&BranchId>,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
) -> String {
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG)
    .bind(unique("receiver"))
    .fetch_one(&db.pool)
    .await
    .expect("insert receiver");
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from, valid_to) \
         VALUES ($1::uuid, $2, $3::uuid, $4, $5, $6)",
    )
    .bind(&id)
    .bind(role.as_str())
    .bind(ORG)
    .bind(branch.map(|b| b.as_str().to_owned()))
    .bind(valid_from)
    .bind(valid_to)
    .execute(&db.pool)
    .await
    .expect("insert receiver role");
    id
}

#[tokio::test]
async fn a_receiver_who_closes_their_own_shift_first_can_then_accept() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let a_shift = opened(&db, &main, &a, &manager).await;
    let b_shift = opened(&db, &main, &b, &manager).await;

    let h = db
        .handovers()
        .start(
            &a,
            StartHandover {
                branch: main.clone(),
                from_shift_id: a_shift.clone(),
                receiver_id: b.user_id.clone(),
                lines: thb_2300(&db).await,
                note: None,
                idempotency_key: unique("handover"),
            },
            at(300),
        )
        .await
        .expect("A starts handover to B");

    refused_with(
        db.handovers().accept(&b, &h.id, at(360)).await,
        &CashError::ShiftAlreadyOpen.to_string(),
    );

    let close = db
        .counts()
        .submit(
            &b,
            SubmitCount {
                branch: main.clone(),
                shift_id: Some(b_shift.clone()),
                kind: CountKind::Close,
                lines: thb_2300(&db).await,
                note: None,
                idempotency_key: unique("close"),
            },
            at(420),
        )
        .await
        .expect("B submits CLOSE count");
    db.counts()
        .verify(&manager, &close.id, at(480))
        .await
        .expect("manager verifies B's CLOSE count");

    assert_eq!(db.shift(&b_shift).await.1, "CLOSED");

    let accepted = db
        .handovers()
        .accept(&b, &h.id, at(540))
        .await
        .expect("B accepts now that own shift is closed");
    assert_eq!(accepted.status, HandoverStatus::Accepted);

    assert_eq!(
        db.shift(&a_shift).await,
        (a.user_id.clone(), "CLOSED".to_owned())
    );
    let b_shifts = db.open_shifts_of(&b).await;
    assert_eq!(b_shifts.len(), 1);
    let b_new_shift = &b_shifts[0].0;
    assert_eq!(Some(b_new_shift), accepted.to_shift_id.as_ref());
    let (opening, expected, _) = db.balance(b_new_shift, "THB").await;
    assert_eq!((opening, expected), (d("2300"), d("2300")));
    assert!(db.open_shifts_of(&a).await.is_empty());
}

#[tokio::test]
async fn the_branchs_own_manager_can_receive_and_then_holds_the_shift() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let a = db.person("Ploy", &[Role::Teller]).await;

    let mgr_id = receiver(&db, Role::BranchManager, Some(&main), at(0), None).await;
    let manager = Actor::new(
        &mgr_id,
        ORG,
        vec![ScopedRole::new(
            Role::BranchManager,
            Some(main.clone()),
            at(0),
            None,
        )],
        None,
    );

    let a_shift = opened(&db, &main, &a, &manager).await;
    assert!(db.open_shifts_of(&manager).await.is_empty());

    let h = db
        .handovers()
        .start(
            &a,
            StartHandover {
                branch: main.clone(),
                from_shift_id: a_shift.clone(),
                receiver_id: manager.user_id.clone(),
                lines: thb_2300(&db).await,
                note: None,
                idempotency_key: unique("handover"),
            },
            at(300),
        )
        .await
        .expect("A starts handover to branch manager");

    let accepted = db
        .handovers()
        .accept(&manager, &h.id, at(360))
        .await
        .expect("branch manager accepts handover");
    assert_eq!(accepted.status, HandoverStatus::Accepted);

    assert_eq!(
        db.shift(&a_shift).await,
        (a.user_id.clone(), "CLOSED".to_owned())
    );
    let mgr_shifts = db.open_shifts_of(&manager).await;
    assert_eq!(mgr_shifts.len(), 1);
    let mgr_shift = &mgr_shifts[0].0;
    assert_eq!(Some(mgr_shift), accepted.to_shift_id.as_ref());
    assert_eq!(
        db.shift(mgr_shift).await,
        (manager.user_id.clone(), "OPEN".to_owned())
    );
    let (opening, expected, _) = db.balance(mgr_shift, "THB").await;
    assert_eq!((opening, expected), (d("2300"), d("2300")));
    assert!(db.open_shifts_of(&a).await.is_empty());
}

#[tokio::test]
async fn a_teller_moved_in_with_a_dated_role_here_can_receive_and_accept() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let other = db.branch("other").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let a_shift = opened(&db, &main, &a, &manager).await;

    let m_id = receiver(&db, Role::Teller, Some(&other), at(NOW - 30 * DAY), None).await;
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from, valid_to) \
         VALUES ($1::uuid, $2, $3::uuid, $4, $5, $6)",
    )
    .bind(&m_id)
    .bind(Role::Teller.as_str())
    .bind(ORG)
    .bind(main.as_str())
    .bind(at(NOW - DAY))
    .bind(Some(at(NOW + DAY)))
    .execute(&db.pool)
    .await
    .expect("insert dated role at this branch");

    let m = Actor::new(
        &m_id,
        ORG,
        vec![
            ScopedRole::new(Role::Teller, Some(other.clone()), at(NOW - 30 * DAY), None),
            ScopedRole::new(
                Role::Teller,
                Some(main.clone()),
                at(NOW - DAY),
                Some(at(NOW + DAY)),
            ),
        ],
        None,
    );

    let h = db
        .handovers()
        .start(
            &a,
            StartHandover {
                branch: main.clone(),
                from_shift_id: a_shift.clone(),
                receiver_id: m.user_id.clone(),
                lines: thb_2300(&db).await,
                note: None,
                idempotency_key: unique("handover"),
            },
            at(NOW),
        )
        .await
        .expect("A hands over to M");

    let accepted = db
        .handovers()
        .accept(&m, &h.id, at(NOW + 60))
        .await
        .expect("M accepts");
    assert_eq!(accepted.status, HandoverStatus::Accepted);

    assert_eq!(
        db.shift(&a_shift).await,
        (a.user_id.clone(), "CLOSED".to_owned())
    );
    let m_shifts = db.open_shifts_of(&m).await;
    assert_eq!(
        m_shifts,
        vec![(
            accepted.to_shift_id.clone().unwrap(),
            main.as_str().to_owned()
        )]
    );
    let m_shift = &m_shifts[0].0;
    assert_eq!(
        db.shift(m_shift).await,
        (m.user_id.clone(), "OPEN".to_owned())
    );
    let (opening, expected, _) = db.balance(m_shift, "THB").await;
    assert_eq!((opening, expected), (d("2300"), d("2300")));
    assert!(db.open_shifts_of(&a).await.is_empty());
}

#[tokio::test]
async fn once_the_moved_in_role_has_ended_the_hand_over_is_refused() {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let other = db.branch("other").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let a_shift = opened(&db, &main, &a, &manager).await;

    let m_id = receiver(&db, Role::Teller, Some(&other), at(NOW - 30 * DAY), None).await;
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from, valid_to) \
         VALUES ($1::uuid, $2, $3::uuid, $4, $5, $6)",
    )
    .bind(&m_id)
    .bind(Role::Teller.as_str())
    .bind(ORG)
    .bind(main.as_str())
    .bind(at(NOW - 2 * DAY))
    .bind(Some(at(NOW - DAY)))
    .execute(&db.pool)
    .await
    .expect("insert expired role at this branch");

    let res = db
        .handovers()
        .start(
            &a,
            StartHandover {
                branch: main.clone(),
                from_shift_id: a_shift.clone(),
                receiver_id: m_id,
                lines: thb_2300(&db).await,
                note: None,
                idempotency_key: unique("handover"),
            },
            at(NOW),
        )
        .await;

    match res {
        Err(AppError::Validation(msg)) => assert_eq!(msg, INVALID_RECEIVER),
        other => panic!("expected Validation(INVALID_RECEIVER), got {other:?}"),
    }
    assert_eq!(db.handover_rows().await, 0);
    assert_eq!(db.shift(&a_shift).await.1, "OPEN");
}
