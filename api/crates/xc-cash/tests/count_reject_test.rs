//! Staff corrections §4 B, rows B1–B9 (`docs/design/staff-corrections.md` SC-D6..SC-D9, M-SC1):
//! `CashCountService::reject_in`, against a real PostgreSQL.
//!
//! Written before the implementation by the sub-controller (SC-0, claude/studio-claude-4). Do not edit, delete or
//! weaken these tests; add your own in another file. Needs `TEST_DATABASE_URL` (a database whose name ends in
//! `_test`); nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Refusal, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CashCount, CountKind, CountLine, CountStatus, RejectError};
use xc_cash::domain::error::CashError;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

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

struct Db {
    pool: PgPool,
    #[allow(dead_code)]
    migrator: Migrator,
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
        let schema = unique("count_reject_test");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        migrator
            .run(&pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        Db { pool, migrator }
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&id)
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
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
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
                 VALUES ($1::uuid, $2, $3::uuid, NULL)",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("insert role");
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
        .unwrap_or_else(|e| panic!("denomination {currency} {face}: {e}"))
    }

    /// A person whose roles cover only `branch` (a branch-scoped `user_roles` row).
    async fn person_at(&self, name: &str, role: Role, branch: &BranchId) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
             VALUES ($1::uuid, $2, $3::uuid, $4)",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch.as_str())
        .execute(&self.pool)
        .await
        .expect("insert a branch role");
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            vec![ScopedRole::new(role, Some(branch.clone()), from, None)],
            None,
        )
    }

    async fn reject(
        &self,
        who: &Actor,
        count_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<CashCount, RejectError> {
        let mut tx = self.pool.begin().await.expect("begin");
        let answer = self
            .counts()
            .reject_in(&mut tx, who, count_id, reason, now)
            .await;
        if answer.is_ok() {
            tx.commit().await.expect("commit");
        }
        answer
    }

    /// `rejected_by`, `rejected_at`, `reject_reason`, `verified_by` of a count, as stored.
    async fn stored(
        &self,
        id: &str,
    ) -> (
        Option<String>,
        Option<DateTime<Utc>>,
        Option<String>,
        Option<String>,
    ) {
        sqlx::query_as(
            "SELECT rejected_by::text, rejected_at, reject_reason, verified_by::text \
             FROM cash_counts WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("read the count")
    }

    /// The ids the `WAITING` list would show for `branch` (no verifier, no rejecter).
    async fn waiting(&self, branch: &BranchId) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT id::text FROM cash_counts \
             WHERE branch_id = $1 AND verified_by IS NULL AND rejected_by IS NULL ORDER BY created_at",
        )
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await
        .expect("read the waiting counts")
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    async fn count_shifts_for_teller(&self, teller_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM shifts WHERE teller_id = $1::uuid")
            .bind(teller_id)
            .fetch_one(&self.pool)
            .await
            .expect("count shifts for teller")
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

/// THB 1000 × 2 + 100 × 3 = 2,300.
async fn thb_2300(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
    ]
}

/// A waiting OPEN count of 2,300 THB by `teller`.
async fn waiting_open(db: &Db, branch: &BranchId, teller: &Actor) -> CashCount {
    let lines = thb_2300(db).await;
    db.counts()
        .submit(teller, cmd(branch, None, CountKind::Open, lines), at(0))
        .await
        .expect("an OPEN count")
}

struct Floor {
    db: Db,
    branch: BranchId,
    teller: Actor,
    manager: Actor,
}

async fn floor() -> Floor {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    Floor {
        db,
        branch,
        teller,
        manager,
    }
}

// B7
#[tokio::test]
async fn a_verifier_rejects_a_waiting_count() {
    let f = floor().await;
    let count = waiting_open(&f.db, &f.branch, &f.teller).await;
    assert_eq!(count.status(), CountStatus::Waiting);
    assert_eq!(f.db.waiting(&f.branch).await, vec![count.id.clone()]);

    let rejected =
        f.db.reject(&f.manager, &count.id, "  นับซ้ำไม่ได้ เงินในตู้ไม่พอ  ", at(60))
            .await
            .expect("the BRANCH_MANAGER rejects the waiting count");

    assert_eq!(rejected.id, count.id);
    assert_eq!(rejected.status(), CountStatus::Rejected);
    let rejection = rejected.rejection.clone().expect("the rejection");
    assert_eq!(rejection.by, f.manager.user_id);
    assert_eq!(rejection.at, at(60));
    assert_eq!(
        rejection.reason, "นับซ้ำไม่ได้ เงินในตู้ไม่พอ",
        "the reason is stored trimmed"
    );
    assert_eq!(rejected.verified_by, None);

    let (by, when, reason, verified_by) = f.db.stored(&count.id).await;
    assert_eq!(by.as_deref(), Some(f.manager.user_id.as_str()));
    assert_eq!(when, Some(at(60)));
    assert_eq!(reason.as_deref(), Some("นับซ้ำไม่ได้ เงินในตู้ไม่พอ"));
    assert_eq!(verified_by, None);
    assert!(
        f.db.waiting(&f.branch).await.is_empty(),
        "it leaves the waiting list"
    );
    assert_eq!(
        f.db.count_shifts_for_teller(&f.teller.user_id).await,
        0,
        "rejecting an OPEN count opens no shift"
    );
}

// B1: no ShiftCloseCertify (a teller) → roleNotAllowed; a manager of another branch → outOfScope.
#[tokio::test]
async fn without_the_verify_permission_on_the_branch_it_is_refused() {
    let f = floor().await;
    let count = waiting_open(&f.db, &f.branch, &f.teller).await;
    let other_teller = f.db.person("Nok", &[Role::Teller]).await;
    let elsewhere = f.db.branch("elsewhere").await;
    let other_manager = f.db.person_at("Som", Role::BranchManager, &elsewhere).await;

    match f.db.reject(&other_teller, &count.id, "why", at(60)).await {
        Err(e @ RejectError::Refused(Refusal::RoleNotAllowed)) => {
            assert_eq!(e.code(), "roleNotAllowed")
        }
        other => panic!("a teller: expected Refused(RoleNotAllowed), got {other:?}"),
    }
    match f.db.reject(&other_manager, &count.id, "why", at(60)).await {
        Err(e @ RejectError::Refused(Refusal::OutOfScope)) => assert_eq!(e.code(), "outOfScope"),
        other => panic!("another branch's manager: expected Refused(OutOfScope), got {other:?}"),
    }
    assert_eq!(
        f.db.stored(&count.id).await,
        (None, None, None, None),
        "unchanged"
    );
}

// B2
#[tokio::test]
async fn an_unknown_count_is_not_found() {
    let f = floor().await;
    let missing = "00000000-0000-0000-0000-00000000dead";
    match f.db.reject(&f.manager, missing, "why", at(60)).await {
        Err(e @ RejectError::NotFound) => assert_eq!(e.code(), "notFound"),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

// B3: a handover count settles through its handover.
#[tokio::test]
async fn a_handover_count_is_only_through_handover() {
    let f = floor().await;
    let count = waiting_open(&f.db, &f.branch, &f.teller).await;
    sqlx::query("UPDATE cash_counts SET kind = 'HANDOVER_OUT', shift_template_id = NULL WHERE id = $1::uuid")
        .bind(&count.id)
        .execute(&f.db.pool)
        .await
        .expect("make it a handover count");
    match f.db.reject(&f.manager, &count.id, "why", at(60)).await {
        Err(e @ RejectError::OnlyThroughHandover) => assert_eq!(e.code(), "onlyThroughHandover"),
        other => panic!("expected OnlyThroughHandover, got {other:?}"),
    }
    assert_eq!(f.db.stored(&count.id).await, (None, None, None, None));
}

// B4: whoever counted it cannot reject it, even holding ShiftCloseCertify.
#[tokio::test]
async fn the_counter_cannot_reject_their_own_count() {
    let f = floor().await;
    let both =
        f.db.person("Dao", &[Role::Teller, Role::BranchManager])
            .await;
    let count = waiting_open(&f.db, &f.branch, &both).await;
    match f.db.reject(&both, &count.id, "why", at(60)).await {
        Err(e @ RejectError::SameCounter) => assert_eq!(e.code(), "sameCounter"),
        other => panic!("expected SameCounter, got {other:?}"),
    }
    assert_eq!(f.db.stored(&count.id).await, (None, None, None, None));
}

// B5: a verified count and a rejected count are no longer waiting.
#[tokio::test]
async fn a_verified_or_rejected_count_is_not_waiting() {
    let f = floor().await;
    let verified = waiting_open(&f.db, &f.branch, &f.teller).await;
    f.db.counts()
        .verify(&f.manager, &verified.id, at(30))
        .await
        .expect("verify it");
    match f.db.reject(&f.manager, &verified.id, "why", at(60)).await {
        Err(e @ RejectError::NotWaiting) => assert_eq!(e.code(), "notWaiting"),
        other => panic!("verified: expected NotWaiting, got {other:?}"),
    }

    let other_teller = f.db.person("Nok", &[Role::Teller]).await;
    let count = waiting_open(&f.db, &f.branch, &other_teller).await;
    f.db.reject(&f.manager, &count.id, "first", at(60))
        .await
        .expect("the first rejection");
    match f.db.reject(&f.manager, &count.id, "second", at(90)).await {
        Err(e @ RejectError::NotWaiting) => assert_eq!(e.code(), "notWaiting"),
        other => panic!("rejected: expected NotWaiting, got {other:?}"),
    }
    let (_, when, reason, _) = f.db.stored(&count.id).await;
    assert_eq!(
        (when, reason.as_deref()),
        (Some(at(60)), Some("first")),
        "the first rejection stays"
    );
}

// B6 (SC-D10)
#[tokio::test]
async fn the_reason_is_required_and_at_most_500_units() {
    let f = floor().await;
    let count = waiting_open(&f.db, &f.branch, &f.teller).await;
    for blank in ["", "   ", "\n\t"] {
        match f.db.reject(&f.manager, &count.id, blank, at(60)).await {
            Err(e @ RejectError::ReasonRequired) => assert_eq!(e.code(), "reasonRequired"),
            other => panic!("{blank:?}: expected ReasonRequired, got {other:?}"),
        }
    }
    match f
        .db
        .reject(&f.manager, &count.id, &"ก".repeat(501), at(60))
        .await
    {
        Err(e @ RejectError::ReasonTooLong) => assert_eq!(e.code(), "validationFailed"),
        other => panic!("501 units: expected ReasonTooLong, got {other:?}"),
    }
    assert_eq!(f.db.stored(&count.id).await, (None, None, None, None));
    f.db.reject(&f.manager, &count.id, &"ก".repeat(500), at(60))
        .await
        .expect("500 units are allowed");
}

// The order of B1–B6: the first fault wins.
#[tokio::test]
async fn the_first_fault_wins() {
    let f = floor().await;
    // not found before anything else, even for a teller with a blank reason
    match f
        .db
        .reject(
            &f.teller,
            "00000000-0000-0000-0000-00000000dead",
            "",
            at(60),
        )
        .await
    {
        Err(RejectError::NotFound) => {}
        other => panic!("expected NotFound first, got {other:?}"),
    }
    let count = waiting_open(&f.db, &f.branch, &f.teller).await;
    // permission before same-counter and reason: the counter is a teller without the permission
    match f.db.reject(&f.teller, &count.id, "", at(60)).await {
        Err(RejectError::Refused(Refusal::RoleNotAllowed)) => {}
        other => panic!("expected Refused first, got {other:?}"),
    }
    // not-waiting before reason
    f.db.reject(&f.manager, &count.id, "first", at(60))
        .await
        .expect("reject");
    match f.db.reject(&f.manager, &count.id, "", at(90)).await {
        Err(RejectError::NotWaiting) => {}
        other => panic!("expected NotWaiting before ReasonRequired, got {other:?}"),
    }
}

// B8: verify refuses a rejected count with notWaiting; a verified count keeps today's answer.
#[tokio::test]
async fn verify_refuses_a_rejected_count() {
    let f = floor().await;
    let count = waiting_open(&f.db, &f.branch, &f.teller).await;
    f.db.reject(&f.manager, &count.id, "why", at(60))
        .await
        .expect("reject");
    match f.db.counts().verify(&f.manager, &count.id, at(90)).await {
        Err(AppError::Domain(message)) => assert_eq!(message, CashError::NotWaiting.to_string()),
        other => panic!("expected Domain(NotWaiting), got {other:?}"),
    }
    assert_eq!(CashError::NotWaiting.code(), "notWaiting");
    assert_eq!(f.db.count_shifts_for_teller(&f.teller.user_id).await, 0);

    let other_teller = f.db.person("Nok", &[Role::Teller]).await;
    let verified = waiting_open(&f.db, &f.branch, &other_teller).await;
    f.db.counts()
        .verify(&f.manager, &verified.id, at(30))
        .await
        .expect("verify");
    match f.db.counts().verify(&f.manager, &verified.id, at(40)).await {
        Err(AppError::Conflict(message)) => assert_eq!(message, "การนับนี้ได้รับการรับรองแล้ว"),
        other => panic!("a verified count keeps today's Conflict, got {other:?}"),
    }
}

// B9 (SC-D9): after a rejection the counter counts again, and the new count verifies.
#[tokio::test]
async fn the_counter_counts_again_after_a_rejection() {
    let f = floor().await;
    let first = waiting_open(&f.db, &f.branch, &f.teller).await;
    f.db.reject(&f.manager, &first.id, "miscounted", at(60))
        .await
        .expect("reject");
    let lines = thb_2300(&f.db).await;
    let again =
        f.db.counts()
            .submit(
                &f.teller,
                cmd(&f.branch, None, CountKind::Open, lines),
                at(120),
            )
            .await
            .expect("the counter submits again");
    assert_eq!(f.db.waiting(&f.branch).await, vec![again.id.clone()]);
    let verified =
        f.db.counts()
            .verify(&f.manager, &again.id, at(180))
            .await
            .expect("the new count verifies");
    assert_eq!(verified.status(), CountStatus::Verified);
    assert!(verified.shift_id.is_some(), "it opens the shift");
}

// M-SC1's three CHECKs refuse a bad direct write.
#[tokio::test]
async fn the_migration_checks_refuse_a_bad_row() {
    let f = floor().await;
    let count = waiting_open(&f.db, &f.branch, &f.teller).await;
    let bad = [
        // rejected_complete: a rejecter without a time and reason
        format!(
            "UPDATE cash_counts SET rejected_by = '{}'::uuid WHERE id = '{}'::uuid",
            f.manager.user_id, count.id
        ),
        // rejecter_not_counter
        format!(
            "UPDATE cash_counts SET rejected_by = counted_by, rejected_at = now(), reject_reason = 'x' WHERE id = '{}'::uuid",
            count.id
        ),
        // verified_or_rejected
        format!(
            "UPDATE cash_counts SET verified_by = '{m}'::uuid, verified_at = now(), rejected_by = '{m}'::uuid, \
             rejected_at = now(), reject_reason = 'x' WHERE id = '{c}'::uuid",
            m = f.manager.user_id,
            c = count.id
        ),
    ];
    for sql in bad {
        let res = sqlx::query(AssertSqlSafe(sql.clone()))
            .execute(&f.db.pool)
            .await;
        assert!(res.is_err(), "the database refuses: {sql}");
    }
    assert_eq!(f.db.stored(&count.id).await, (None, None, None, None));
}
