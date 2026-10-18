//! Cash count application service.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Role, ScopedRole, Target};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MoneyDelta};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
use xc_shift::infrastructure::custody::custody_repository::{
    append_cash_exception_in, day_ready_on_open_count_in,
};
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::application::ledger_approval::count_difference::{
    COUNT_DIFFERENCE, CountDifferenceGate, GateResult,
};
use crate::domain::count::{CashCount, CountKind, CountLine};
use crate::domain::error::CashError;
use crate::domain::ledger::posting::{SourceRef, SourceType};
use crate::domain::person_rules::PersonRules;
use crate::domain::posting::{CashPlace, CashPostings, differences, moves};
use crate::domain::repository::{CashRepository, CountTotal};
use crate::infrastructure::inventory::repository::PgInventoryRepository;
use crate::infrastructure::ledger_postings::LedgerCashPostings;
use crate::infrastructure::repository::PgCashRepository;

#[derive(Debug, Clone)]
pub struct SubmitCount {
    pub branch: BranchId,
    pub shift_id: Option<String>,
    pub kind: CountKind,
    pub lines: Vec<CountLine>,
    pub note: Option<String>,
    pub idempotency_key: String,
}

#[derive(Clone)]
pub struct CashCountService {
    pool: PgPool,
    repo: Arc<PgCashRepository>,
    postings: Arc<dyn CashPostings>,
    shifts: ShiftService<PgShiftRepository>,
    gate: Arc<CountDifferenceGate>,
    approvals_repo: Arc<PgApprovalRepository>,
}

impl CashCountService {
    pub fn new(
        pool: PgPool,
        repo: Arc<PgCashRepository>,
        postings: Arc<dyn CashPostings>,
        shifts: ShiftService<PgShiftRepository>,
        gate: Arc<CountDifferenceGate>,
        approvals_repo: Arc<PgApprovalRepository>,
    ) -> Self {
        Self {
            pool,
            repo,
            postings,
            shifts,
            gate,
            approvals_repo,
        }
    }

    pub fn pg(pool: PgPool) -> Self {
        let repo = Arc::new(PgCashRepository::new(pool.clone()));
        let postings: Arc<dyn CashPostings> = Arc::new(LedgerCashPostings::new());
        let shift_repo = Arc::new(
            crate::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                pool.clone(),
            ),
        );
        let transactor = Transactor::new(pool.clone());
        let branch_tx = BranchTx::new(transactor);
        let shifts = ShiftService::new(shift_repo, branch_tx);

        let mut registry = ApprovalRegistry::new();
        crate::application::ledger_approval::count_difference::register(
            &mut registry,
            pool.clone(),
        );
        let approvals_repo = Arc::new(PgApprovalRepository::new(pool.clone()));
        let audit_log = Arc::new(PgAuditLog::new(pool.clone()));
        let approvals = Arc::new(ApprovalService::new(
            registry,
            approvals_repo.clone(),
            audit_log,
        ));
        let inventory = Arc::new(PgInventoryRepository::new(pool.clone()));
        let gate = Arc::new(CountDifferenceGate::new(approvals, inventory));

        Self::new(pool, repo, postings, shifts, gate, approvals_repo)
    }

    pub fn postings(&self) -> &Arc<dyn CashPostings> {
        &self.postings
    }

    /// A drawer's balances in the ledger (L-4: balances are derived from the ledger), as
    /// (currency, amount) for each currency it holds.
    pub async fn drawer_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        shift_id: &str,
    ) -> AppResult<Vec<(String, Decimal)>> {
        let held = self
            .postings
            .expected_in(tx, &CashPlace::Drawer(shift_id.to_owned()))
            .await?;
        Ok(held
            .non_zero()
            .into_iter()
            .map(|(c, m)| {
                (
                    c.as_str().trim().to_owned(),
                    Decimal::from_i128_with_scale(m.minor_i128(), 2),
                )
            })
            .collect())
    }

    pub fn repo(&self) -> &Arc<PgCashRepository> {
        &self.repo
    }

    pub fn shifts(&self) -> &ShiftService<PgShiftRepository> {
        &self.shifts
    }

    pub async fn submit(
        &self,
        actor: &Actor,
        cmd: SubmitCount,
        now: DateTime<Utc>,
    ) -> AppResult<CashCount> {
        let mut tx = self.pool.begin().await?;
        let count = self.submit_in(&mut tx, actor, cmd, now).await?;
        tx.commit().await?;
        Ok(count)
    }

    /// Verifies a count. Awaiting FINANCE commits the request and answers
    /// `Err(CashError::AwaitingFinance(id))`, as before CA-7c; `verify_in` answers it as a value.
    pub async fn verify(
        &self,
        actor: &Actor,
        count_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashCount> {
        let mut tx = self.pool.begin().await?;
        match self.verify_in(&mut tx, actor, count_id, now).await? {
            CountVerified::Verified(count) => {
                tx.commit().await?;
                Ok(*count)
            }
            CountVerified::AwaitingFinance(request_id) => {
                tx.commit().await?;
                Err(CashError::AwaitingFinance(request_id).into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::str::FromStr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::{TimeZone, Utc};
    use pos_error::AppError;
    use rust_decimal::Decimal;
    use sqlx::migrate::Migrator;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use sqlx::{AssertSqlSafe, PgPool};
    use xc_auth::domain::access::{Actor, Role, ScopedRole};
    use xc_kernel::ids::BranchId;
    use xc_kernel::money::Money;
    use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta};

    use super::*;

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

    fn migrations_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
    }

    struct TestDb {
        pool: PgPool,
    }

    impl TestDb {
        async fn fresh() -> Self {
            let url = std::env::var("TEST_DATABASE_URL")
                .unwrap_or_else(|_| panic!("TEST_DATABASE_URL is not set"));
            let options = PgConnectOptions::from_str(&url).expect("postgres options");
            let schema = unique("cs_test");
            let admin = PgPoolOptions::new()
                .max_connections(1)
                .connect_with(options.clone())
                .await
                .expect("admin connect");
            sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
                .execute(&admin)
                .await
                .expect("create schema");
            admin.close().await;

            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect_with(options.options([("search_path", schema.as_str())]))
                .await
                .expect("connect schema pool");

            let migrator = Migrator::new(migrations_dir().as_path())
                .await
                .expect("migrator");
            migrator.run(&pool).await.expect("migrations");

            TestDb { pool }
        }

        async fn branch(&self, label: &str) -> BranchId {
            let id = unique(label);
            sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
                .bind(&id)
                .execute(&self.pool)
                .await
                .expect("insert branch");
            let branch = BranchId::new(id);
            // LG-4 (#929, lane controller's 1:1 ruling): an OPEN count's cash now leaves the branch's ledger SAFE
            // (design §4.4), so the safe holds 1,000,000,000 of every register currency (`OPENING` from BANK, one
            // posting per currency). Before the cutover nothing had to be there.
            let codes: Vec<String> =
                sqlx::query_scalar("SELECT trim(code) FROM currencies ORDER BY 1")
                    .fetch_all(&self.pool)
                    .await
                    .expect("the currency register");
            let mut tx = self.pool.begin().await.expect("begin");
            for code in codes {
                let request = crate::domain::posting::PostingRequest {
                    from: crate::domain::posting::CashPlace::Bank,
                    to: crate::domain::posting::CashPlace::Safe(branch.clone()),
                    currency: xc_kernel::money::CurrencyCode::parse(&code).expect("a currency"),
                    quantity: xc_kernel::money::ForeignAmount::from_decimal(Decimal::from(
                        1_000_000_000_i64,
                    )),
                    source: crate::domain::ledger::posting::SourceRef {
                        source_type: crate::domain::ledger::posting::SourceType::Opening,
                        source_id: unique(&format!("lg4-fund-{code}")),
                        source_key: "fund".to_string(),
                    },
                    line: None,
                    reverses: None,
                };
                crate::domain::posting::CashPostings::post_in(
                    &crate::infrastructure::ledger_postings::LedgerCashPostings::new(),
                    &mut tx,
                    &[request],
                    None,
                )
                .await
                .expect("fund the safe through the ledger");
            }
            tx.commit().await.expect("commit");
            branch
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
            .expect("insert user");

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
    }

    #[tokio::test]
    async fn line_whose_denomination_belongs_to_another_currency_is_refused_and_usd_total_is_unchanged()
     {
        let db = TestDb::fresh().await;
        let branch = db.branch("main").await;
        let teller = db.person("Ploy", &[Role::Teller]).await;
        let manager = db.person("Krit", &[Role::BranchManager]).await;

        let usd_denom: i64 = sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('USD', 100, 'TEST') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .expect("insert usd denom");

        let jpy_denom: i64 = sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('JPY', 10000, 'TEST') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .expect("insert jpy denom");

        let service = CashCountService::pg(db.pool.clone());
        let now = Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap();

        // First submit and verify a valid USD count of 10 notes = 1000 USD
        let initial_cmd = SubmitCount {
            branch: branch.clone(),
            shift_id: None,
            kind: CountKind::Open,
            lines: vec![CountLine::new("USD", usd_denom, 10)],
            note: None,
            idempotency_key: unique("init_usd"),
        };
        let init_count = service
            .submit(&teller, initial_cmd, now)
            .await
            .expect("initial submit");
        let verified = service
            .verify(&manager, &init_count.id, now)
            .await
            .expect("initial verify");

        let usd_balance_before: Decimal = sqlx::query_scalar(
            "SELECT opening FROM shift_balances WHERE shift_id = $1 AND code = 'USD'",
        )
        .bind(verified.shift_id.as_deref().unwrap())
        .fetch_one(&db.pool)
        .await
        .expect("fetch usd balance before");
        assert_eq!(usd_balance_before, Decimal::from(1000));

        // Now attempt to submit a count where a JPY denomination is sent as "USD"
        let bad_cmd = SubmitCount {
            branch: branch.clone(),
            shift_id: verified.shift_id.clone(),
            kind: CountKind::Close,
            lines: vec![
                CountLine::new("USD", usd_denom, 10),
                CountLine::new("USD", jpy_denom, 1),
            ],
            note: None,
            idempotency_key: unique("mismatch_usd"),
        };

        let res = service.submit(&teller, bad_cmd, now).await;
        assert!(
            res.is_err(),
            "mismatched denomination currency must be refused"
        );
        match res {
            Err(AppError::Validation(msg)) => {
                assert!(msg.contains("does not match denomination currency"));
            }
            other => panic!("expected AppError::Validation, got {other:?}"),
        }

        // Check USD total is unchanged on shift_balances
        let usd_balance_after: Decimal = sqlx::query_scalar(
            "SELECT opening FROM shift_balances WHERE shift_id = $1 AND code = 'USD'",
        )
        .bind(verified.shift_id.as_deref().unwrap())
        .fetch_one(&db.pool)
        .await
        .expect("fetch usd balance after");
        assert_eq!(usd_balance_after, Decimal::from(1000));

        let count_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM cash_counts WHERE idempotency_key = $1")
                .bind("mismatch_usd")
                .fetch_one(&db.pool)
                .await
                .expect("count rows");
        assert_eq!(count_rows, 0);
    }

    #[tokio::test]
    async fn kwd_total_with_three_decimal_places_is_refused_at_submit_and_nothing_is_written() {
        let db = TestDb::fresh().await;
        let branch = db.branch("main").await;
        let teller = db.person("Ploy", &[Role::Teller]).await;

        let kwd_denom: i64 = sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('KWD', 0.005, 'TEST') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .expect("insert kwd denom");

        let service = CashCountService::pg(db.pool.clone());
        let open_cmd = SubmitCount {
            branch: branch.clone(),
            shift_id: None,
            kind: CountKind::Open,
            lines: vec![CountLine::new("KWD", kwd_denom, 301)], // 301 * 0.005 = 1.505
            note: None,
            idempotency_key: unique("open_kwd_3dec"),
        };

        let now = Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap();
        let res = service.submit(&teller, open_cmd, now).await;
        assert!(
            res.is_err(),
            "3-decimal-place total must be refused with TooManyDecimals"
        );
        match res {
            Err(AppError::Domain(msg)) => {
                assert_eq!(msg, CashError::TooManyDecimals.to_string());
            }
            other => panic!("expected TooManyDecimals domain error, got {other:?}"),
        }

        let count_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM cash_counts WHERE branch_id = $1")
                .bind(branch.as_str())
                .fetch_one(&db.pool)
                .await
                .expect("count rows");
        assert_eq!(count_rows, 0, "nothing written to cash_counts");

        let line_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM cash_count_lines")
            .fetch_one(&db.pool)
            .await
            .expect("count lines");
        assert_eq!(line_rows, 0, "nothing written to cash_count_lines");

        let total_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM cash_count_totals")
            .fetch_one(&db.pool)
            .await
            .expect("count totals");
        assert_eq!(total_rows, 0, "nothing written to cash_count_totals");
    }

    #[tokio::test]
    async fn large_total_over_100_million_is_kept_on_shift_and_exchange_moves_it() {
        let db = TestDb::fresh().await;
        let branch = db.branch("main").await;
        let teller = db.person("Ploy", &[Role::Teller]).await;
        let manager = db.person("Krit", &[Role::BranchManager]).await;

        let krw_denom: i64 = sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('KRW', 50000, 'TEST') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .expect("insert krw denom");

        let service = CashCountService::pg(db.pool.clone());
        let open_cmd = SubmitCount {
            branch: branch.clone(),
            shift_id: None,
            kind: CountKind::Open,
            lines: vec![CountLine::new("KRW", krw_denom, 3000)], // 3000 * 50000 = 150,000,000
            note: None,
            idempotency_key: unique("open_large_krw"),
        };

        let now = Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap();
        let count = service
            .submit(&teller, open_cmd, now)
            .await
            .expect("submit count");
        let verified = service
            .verify(&manager, &count.id, now)
            .await
            .expect("verify count");
        let shift_id = verified.shift_id.expect("shift id");

        let opening: Decimal = sqlx::query_scalar(
            "SELECT opening FROM shift_balances WHERE shift_id = $1 AND code = 'KRW'",
        )
        .bind(&shift_id)
        .fetch_one(&db.pool)
        .await
        .expect("fetch shift balance");

        assert_eq!(
            opening,
            Decimal::from(150_000_000),
            "150M KRW carried into shift_balances at open"
        );

        // Test Must-fix 1: apply +100 exchange on KRW 150,000,000 balance
        let delta_plus_100 = StockDelta::for_movement(
            CurrencyCode::parse("KRW").unwrap(),
            Direction::In,
            Money::parse("100").unwrap(),
        );
        let mut tx = db.pool.begin().await.unwrap();
        let res_plus = service
            .shifts()
            .apply_delta_for_teller_in(&mut tx, &branch, &teller.user_id, &delta_plus_100)
            .await;
        assert!(res_plus.is_ok(), "apply +100 delta succeeds");
        tx.commit().await.unwrap();

        let exp_after_plus: Decimal = sqlx::query_scalar(
            "SELECT expected FROM shift_balances WHERE shift_id = $1 AND code = 'KRW'",
        )
        .bind(&shift_id)
        .fetch_one(&db.pool)
        .await
        .expect("fetch expected after +100");
        assert_eq!(
            exp_after_plus,
            Decimal::from(150_000_100),
            "+100 exchange keeps the 150M balance"
        );

        // Apply -1000 exchange on KRW 150,000,100 balance
        let delta_minus_1000 = StockDelta::for_movement(
            CurrencyCode::parse("KRW").unwrap(),
            Direction::Out,
            Money::parse("1000").unwrap(),
        );
        let mut tx = db.pool.begin().await.unwrap();
        let res_minus = service
            .shifts()
            .apply_delta_for_teller_in(&mut tx, &branch, &teller.user_id, &delta_minus_1000)
            .await;
        assert!(
            res_minus.is_ok(),
            "-1000 exchange on 150M balance is accepted, not refused as insufficient"
        );
        tx.commit().await.unwrap();

        let exp_after_minus: Decimal = sqlx::query_scalar(
            "SELECT expected FROM shift_balances WHERE shift_id = $1 AND code = 'KRW'",
        )
        .bind(&shift_id)
        .fetch_one(&db.pool)
        .await
        .expect("fetch expected after -1000");
        assert_eq!(exp_after_minus, Decimal::from(150_000_100 - 1000));
    }

    #[tokio::test]
    async fn missing_currencies_row_fails_closed_with_internal_error() {
        let db = TestDb::fresh().await;
        let branch = db.branch("main").await;
        let teller = db.person("Ploy", &[Role::Teller]).await;

        sqlx::query("ALTER TABLE denominations DROP CONSTRAINT denominations_currency_code_fkey")
            .execute(&db.pool)
            .await
            .expect("drop fk constraint");

        let zzz_denom: i64 = sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('ZZZ', 100, 'TEST') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .expect("insert zzz denom");

        let service = CashCountService::pg(db.pool.clone());
        let cmd = SubmitCount {
            branch: branch.clone(),
            shift_id: None,
            kind: CountKind::Open,
            lines: vec![CountLine::new("ZZZ", zzz_denom, 1)],
            note: None,
            idempotency_key: unique("unregistered"),
        };

        let now = Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap();
        let res = service.submit(&teller, cmd.clone(), now).await;
        assert!(res.is_err());
        match res {
            Err(AppError::Internal(msg)) => {
                assert!(msg.contains("currency not found in register"));
            }
            other => panic!("expected AppError::Internal, got {other:?}"),
        }

        let count_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM cash_counts WHERE branch_id = $1")
                .bind(branch.as_str())
                .fetch_one(&db.pool)
                .await
                .expect("count rows");
        assert_eq!(count_rows, 0, "nothing written to cash_counts");
    }

    #[tokio::test]
    async fn close_count_omitting_held_currency_gives_difference_minus_expected_and_one_exception_row()
     {
        let db = TestDb::fresh().await;
        let branch = db.branch("main").await;
        let teller = db.person("Ploy", &[Role::Teller]).await;
        let manager = db.person("Krit", &[Role::BranchManager]).await;

        let usd_denom: i64 = sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('USD', 100, 'TEST') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .expect("insert usd denom");

        let thb_denom: i64 = sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('THB', 500, 'TEST') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .expect("insert thb denom");

        let service = CashCountService::pg(db.pool.clone());
        let now = Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap();

        // V-1 (FINANCE approval above limit) / #928: branch limit covering the -1000 THB difference
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, 1000.00, 'test', now())",
        )
        .bind(ORG)
        .bind(branch.as_str())
        .execute(&db.pool)
        .await
        .expect("insert cash limit");

        // Open shift with 5 * 100 USD = 500 USD and 2 * 500 THB = 1000 THB
        let open_cmd = SubmitCount {
            branch: branch.clone(),
            shift_id: None,
            kind: CountKind::Open,
            lines: vec![
                CountLine::new("USD", usd_denom, 5),
                CountLine::new("THB", thb_denom, 2),
            ],
            note: None,
            idempotency_key: unique("open_multi"),
        };

        let open_count = service
            .submit(&teller, open_cmd, now)
            .await
            .expect("submit open count");
        let verified_open = service
            .verify(&manager, &open_count.id, now)
            .await
            .expect("verify open count");
        let shift_id = verified_open.shift_id.expect("shift id");

        // Submit CLOSE count omitting THB (only counting USD 5 * 100 = 500)
        let close_cmd = SubmitCount {
            branch: branch.clone(),
            shift_id: Some(shift_id.clone()),
            kind: CountKind::Close,
            lines: vec![CountLine::new("USD", usd_denom, 5)],
            note: None,
            idempotency_key: unique("close_omit_thb"),
        };

        let close_count = service
            .submit(&teller, close_cmd, now)
            .await
            .expect("submit close count");
        let _verified_close = service
            .verify(&manager, &close_count.id, now)
            .await
            .expect("verify close count");

        // Check cash_count_totals for close_count
        let mut tx = db.pool.begin().await.unwrap();
        let totals = service
            .repo()
            .get_count_totals(&mut tx, &close_count.id)
            .await
            .expect("get totals");

        let thb_total = totals
            .iter()
            .find(|t| t.currency == "THB")
            .expect("THB total present");
        assert_eq!(thb_total.counted, Decimal::ZERO);
        assert_eq!(thb_total.expected, Some(Decimal::from(1000)));
        assert_eq!(thb_total.difference, Some(Decimal::from(-1000)));

        let usd_total = totals
            .iter()
            .find(|t| t.currency == "USD")
            .expect("USD total present");
        assert_eq!(usd_total.counted, Decimal::from(500));
        assert_eq!(usd_total.expected, Some(Decimal::from(500)));
        assert_eq!(usd_total.difference, Some(Decimal::ZERO));

        // Check shift_balances
        let (thb_cnt, thb_diff): (Decimal, Decimal) = sqlx::query_as(
            "SELECT counted, difference FROM shift_balances WHERE shift_id = $1 AND code = 'THB'",
        )
        .bind(&shift_id)
        .fetch_one(&db.pool)
        .await
        .expect("fetch thb balance");
        assert_eq!(thb_cnt, Decimal::ZERO);
        assert_eq!(thb_diff, Decimal::from(-1000));

        let (usd_cnt, usd_diff): (Decimal, Decimal) = sqlx::query_as(
            "SELECT counted, difference FROM shift_balances WHERE shift_id = $1 AND code = 'USD'",
        )
        .bind(&shift_id)
        .fetch_one(&db.pool)
        .await
        .expect("fetch usd balance");
        assert_eq!(usd_cnt, Decimal::from(500));
        assert_eq!(usd_diff, Decimal::ZERO);

        // Check cash_exceptions: exactly 1 row, for THB with diff -1000 in CA-7d shape
        let exceptions: Vec<(String, serde_json::Value)> = sqlx::query_as(
            "SELECT branch_id, data FROM cash_exceptions WHERE data->>'requestId' = $1",
        )
        .bind(&close_count.id)
        .fetch_all(&db.pool)
        .await
        .expect("fetch exceptions");

        assert_eq!(exceptions.len(), 1, "exactly one exception row");
        let exc = &exceptions[0].1;
        assert_eq!(exc["status"], "OPEN");
        assert_eq!(exc["note"], format!("COUNT {}", close_count.id));
        assert_eq!(exc["cashVariance"]["THB"], -100000);
        assert!(exc["safeVariance"].is_null());
        assert!(exc["targetDifference"].is_null());
        assert_eq!(exc["at"], now.timestamp_millis());
        assert!(exc["reviewedBy"].is_null());
        assert!(exc["reviewedAt"].is_null());
    }
}

/// What `verify_in` answers (CA-7c #1290, design §4.9): the count verified, or the count's difference waiting for
/// FINANCE under the approval request it recorded. Awaiting FINANCE is a value, not an error, so the request
/// commits with the caller's transaction.
#[derive(Debug, Clone)]
pub enum CountVerified {
    /// Boxed: `CashCount` is far larger than the other variant (`clippy::large_enum_variant`).
    Verified(Box<CashCount>),
    /// The approval request's id.
    AwaitingFinance(String),
}

/// The CA-7c forms (design §4.9): all their work, reads included, on the caller's transaction. They open no
/// transaction and take no lock; the caller (the CM-1 seam, or the wrapper above) holds both.
impl CashCountService {
    pub async fn submit_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        actor: &Actor,
        cmd: SubmitCount,
        now: DateTime<Utc>,
    ) -> AppResult<CashCount> {
        let permission = match cmd.kind {
            CountKind::Open | CountKind::HandoverOut | CountKind::Close => Permission::ShiftOperate,
            CountKind::SafeOpen | CountKind::Safe => Permission::CashManage,
        };
        let target = Target::new(&actor.organization_id, Some(cmd.branch.clone()), None);
        Authorizer::check(Some(actor), permission, &target, now)
            .map_err(|r| AppError::Domain(CashError::Refused(r).to_string()))?;

        if let Some(existing) = self
            .repo
            .find_count_by_idempotency_key(&mut *tx, &cmd.branch, &cmd.idempotency_key)
            .await?
        {
            PersonRules::replay_is_own(&existing.counted_by, actor)
                .map_err(|e| AppError::Conflict(e.to_string()))?;
            return Ok(existing);
        }

        // After the replay lookup, as in handover start: a retry of a CLOSE count whose verification has
        // since closed the shift answers the original count, not NoOpenShift.
        if let Some(ref s_id) = cmd.shift_id {
            let shift_teller: Option<String> = sqlx::query_scalar(
                "SELECT teller_id::text FROM shifts WHERE id = $1 AND status = 'OPEN' AND branch_id = $2",
            )
            .bind(s_id)
            .bind(cmd.branch.as_str())
            .fetch_optional(&mut **tx)
            .await?;
            let teller_id = shift_teller.ok_or(ShiftError::NoOpenShift)?;
            PersonRules::teller_owns_shift(&teller_id, actor)
                .map_err(|e| AppError::Domain(e.to_string()))?;
        }

        let mut totals_map: std::collections::BTreeMap<String, Decimal> =
            std::collections::BTreeMap::new();
        let mut sanitized_lines = Vec::with_capacity(cmd.lines.len());
        let mut seen_denominations: std::collections::HashSet<i64> =
            std::collections::HashSet::with_capacity(cmd.lines.len());

        for line in &cmd.lines {
            if !seen_denominations.insert(line.denomination_id) {
                return Err(AppError::Validation(format!(
                    "duplicate denomination_id in count lines: {}",
                    line.denomination_id
                )));
            }
            if line.qty < 0 {
                return Err(AppError::Validation(format!(
                    "line qty cannot be negative: {}",
                    line.qty
                )));
            }

            let row: Option<(Decimal, String)> =
                sqlx::query_as("SELECT face_value, currency_code FROM denominations WHERE id = $1")
                    .bind(line.denomination_id)
                    .fetch_optional(&mut **tx)
                    .await?;

            let (face_value, denom_currency) = match row {
                Some(r) => r,
                None => {
                    return Err(AppError::NotFound {
                        entity: "denomination",
                        id: line.denomination_id.to_string(),
                    });
                }
            };

            let denom_currency = denom_currency.trim();
            let line_currency = line.currency.trim();
            if !denom_currency.eq_ignore_ascii_case(line_currency) {
                return Err(AppError::Validation(format!(
                    "line currency '{line_currency}' does not match denomination currency '{denom_currency}'"
                )));
            }

            let line_total = Decimal::from(line.qty) * face_value;
            *totals_map
                .entry(denom_currency.to_string())
                .or_insert(Decimal::ZERO) += line_total;

            sanitized_lines.push(CountLine {
                currency: denom_currency.to_string(),
                denomination_id: line.denomination_id,
                qty: line.qty,
            });
        }

        let held = match cmd.shift_id {
            Some(ref s_id) => self.drawer_in(tx, s_id).await?,
            None => Vec::new(),
        };
        if cmd.kind == CountKind::Close {
            for (code, _) in &held {
                totals_map.entry(code.clone()).or_insert(Decimal::ZERO);
            }
        }

        for (curr, counted) in &totals_map {
            let minor_unit: Option<i16> =
                sqlx::query_scalar("SELECT minor_unit FROM currencies WHERE code = $1")
                    .bind(curr)
                    .fetch_optional(&mut **tx)
                    .await?;

            let minor = minor_unit.ok_or_else(|| {
                AppError::Internal(format!("currency not found in register: {curr}"))
            })?;

            let max_scale = (minor as u32).min(2);
            if counted.normalize().scale() > max_scale {
                return Err(CashError::TooManyDecimals.into());
            }
        }

        let mut totals = Vec::new();
        for (curr, counted) in totals_map {
            let expected = held_of(&held, &curr);

            totals.push(CountTotal {
                currency: curr,
                counted,
                expected,
                difference: None,
            });
        }

        let count_id = uuid::Uuid::new_v4().to_string();
        let cash_count = CashCount {
            id: count_id,
            organization_id: actor.organization_id.clone(),
            branch_id: cmd.branch,
            shift_id: cmd.shift_id,
            kind: cmd.kind,
            counted_by: actor.user_id.clone(),
            counted_at: now,
            verified_by: None,
            verified_at: None,
            note: cmd.note,
            idempotency_key: cmd.idempotency_key,
            created_at: now,
            updated_at: now,
        };

        self.repo
            .insert_count(&mut *tx, &cash_count, &sanitized_lines, &totals)
            .await?;

        Ok(cash_count)
    }

    pub async fn verify_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        actor: &Actor,
        count_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CountVerified> {
        let count = self
            .repo
            .find_count_by_id(&mut *tx, count_id)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "cash_count",
                id: count_id.to_string(),
            })?;

        if count.verified_at.is_some() {
            return Err(AppError::Conflict("การนับนี้ได้รับการรับรองแล้ว".into()));
        }

        if count.kind == CountKind::HandoverOut {
            return Err(AppError::Domain(
                crate::domain::handover::HandoverError::OnlyThroughHandover.to_string(),
            ));
        }

        let permission = match count.kind {
            CountKind::HandoverOut => Permission::ShiftOperate,
            _ => Permission::ShiftCloseCertify,
        };
        let target = Target::new(&count.organization_id, Some(count.branch_id.clone()), None);
        Authorizer::check(Some(actor), permission, &target, now)
            .map_err(|r| AppError::Domain(CashError::Refused(r).to_string()))?;

        PersonRules::verifier_is_not_counter(&count.counted_by, actor)
            .map_err(|e| AppError::Domain(e.to_string()))?;

        if !self.repo.is_newest_count(&mut *tx, &count).await? {
            return Err(CashError::NotNewestCount.into());
        }

        if count.kind == CountKind::Open {
            let open_shift_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM shifts \
                 WHERE teller_id = $1::uuid AND status = 'OPEN'",
            )
            .bind(&count.counted_by)
            .fetch_one(&mut **tx)
            .await?;

            if open_shift_count > 0 {
                return Err(CashError::ShiftAlreadyOpen.into());
            }

            let totals = self.repo.get_count_totals(&mut *tx, count_id).await?;
            let mut opening_pairs = Vec::new();
            let mut opening = Vec::new();
            for t in &totals {
                let code = CurrencyCode::parse(t.currency.trim()).map_err(|_| {
                    AppError::Internal(format!("unknown currency code in count: {}", t.currency))
                })?;
                if !t.counted.is_zero() {
                    opening_pairs.push((t.currency.clone(), t.counted));
                    opening.push((code, t.counted));
                }
            }
            day_ready_on_open_count_in(&mut *tx, &count.branch_id).await?;
            let shift = self
                .shifts
                .open_on_verified(
                    &mut *tx,
                    &count.branch_id,
                    &count.counted_by,
                    count_id,
                    &opening_pairs,
                    now,
                )
                .await?;
            // §4.4 row 1: the counted cash leaves the branch's SAFE for the new drawer.
            post_opening_in(
                self.postings.as_ref(),
                tx,
                CashPlace::Safe(count.branch_id.clone()),
                &shift.id,
                &opening,
                count_id,
            )
            .await?;

            self.repo
                .update_count_verified(
                    &mut *tx,
                    count_id,
                    &actor.user_id,
                    now,
                    Some(&shift.id),
                    &totals,
                )
                .await?;

            let mut verified_count = count;
            verified_count.verified_by = Some(actor.user_id.clone());
            verified_count.verified_at = Some(now);
            verified_count.shift_id = Some(shift.id);
            return Ok(CountVerified::Verified(Box::new(verified_count)));
        }

        if count.kind == CountKind::Close {
            let shift_id = count.shift_id.as_deref().ok_or(ShiftError::NoOpenShift)?;
            let totals = self.repo.get_count_totals(&mut *tx, count_id).await?;

            let held = self.drawer_in(tx, shift_id).await?;

            for (code, _exp) in &held {
                if !totals
                    .iter()
                    .any(|t| t.currency.trim().eq_ignore_ascii_case(code))
                {
                    return Err(CashError::BalancesChanged.into());
                }
            }

            for t in &totals {
                let current_exp = held_of(&held, &t.currency);

                if current_exp != t.expected {
                    return Err(CashError::BalancesChanged.into());
                }
            }

            let mut updated_totals = Vec::new();
            for t in totals {
                CurrencyCode::parse(t.currency.trim()).map_err(|_| {
                    AppError::Internal(format!("unknown currency code in count: {}", t.currency))
                })?;
                let exp = t.expected.unwrap_or(Decimal::ZERO);
                let diff = t.counted - exp;
                updated_totals.push(CountTotal {
                    currency: t.currency.clone(),
                    counted: t.counted,
                    expected: t.expected,
                    difference: Some(diff),
                });
            }

            for t in &updated_totals {
                sqlx::query(
                    "UPDATE cash_count_totals \
                     SET difference = $1::numeric \
                     WHERE count_id = $2::uuid AND currency = $3",
                )
                .bind(t.difference)
                .bind(count_id)
                .bind(&t.currency)
                .execute(&mut **tx)
                .await?;
            }

            let key = format!("count-difference:{}", count_id);
            let existing_req = self
                .approvals_repo
                .find_by_type_and_key(&mut *tx, COUNT_DIFFERENCE, &key)
                .await?;

            if let Some(req) = existing_req {
                match req.status.as_str() {
                    "APPROVED" => {}
                    "REJECTED" => {
                        return Err(CashError::RejectedByFinance(req.id.to_string()).into());
                    }
                    "PENDING" => {
                        return Ok(CountVerified::AwaitingFinance(req.id.to_string()));
                    }
                    other => {
                        return Err(AppError::Internal(format!(
                            "unexpected approval request status: {other}"
                        )));
                    }
                }
            } else {
                let by = Actor::new(
                    &actor.user_id,
                    &count.organization_id,
                    vec![ScopedRole::new(
                        Role::BranchManager,
                        Some(count.branch_id.clone()),
                        DateTime::<Utc>::UNIX_EPOCH,
                        None,
                    )],
                    None,
                );
                match self.gate.check_in(&mut *tx, &count, &by).await? {
                    GateResult::Clear => {}
                    GateResult::AwaitingFinance(req_id) => {
                        return Ok(CountVerified::AwaitingFinance(req_id.to_string()));
                    }
                }
            }

            // §4.4 row 7: the difference goes between the drawer and the branch's VARIANCE.
            let mut diffs = Vec::new();
            for t in &updated_totals {
                let code = CurrencyCode::parse(t.currency.trim()).map_err(|_| {
                    AppError::Internal(format!("unknown currency code in count: {}", t.currency))
                })?;
                diffs.push((code, t.difference.unwrap_or(Decimal::ZERO)));
            }
            post_difference_in(
                self.postings.as_ref(),
                tx,
                shift_id,
                &count.branch_id,
                &diffs,
                count_id,
            )
            .await?;

            let mut close_balances = Vec::new();
            let mut delta_pairs = Vec::new();
            for t in &updated_totals {
                let diff = t.difference.unwrap_or(Decimal::ZERO);
                if !diff.is_zero() {
                    let code = CurrencyCode::parse(t.currency.trim()).map_err(|_| {
                        AppError::Internal(format!(
                            "unknown currency code in count: {}",
                            t.currency
                        ))
                    })?;
                    let mut d = diff;
                    d.rescale(2);
                    delta_pairs.push((code, MoneyDelta::from_minor(d.mantissa())));
                }
                close_balances.push((t.currency.clone(), t.counted, diff));
            }

            if !delta_pairs.is_empty() {
                let exc = CashException {
                    id: format!("EX-{}", uuid::Uuid::new_v4()),
                    request_id: count_id.to_string(),
                    status: ExceptionStatus::Open,
                    target_difference: None,
                    cash_variance: Some(StockDelta::of(&delta_pairs)),
                    safe_variance: None,
                    note: format!("COUNT {count_id}"),
                    at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
                    reviewed_by: None,
                    reviewed_at: None,
                };
                append_cash_exception_in(tx, &count.branch_id, &exc).await?;
            }

            self.shifts
                .close_on_verified(
                    &mut *tx,
                    shift_id,
                    &actor.user_id,
                    &close_balances,
                    count.note.as_deref().unwrap_or(""),
                    now,
                )
                .await?;

            self.repo
                .update_count_verified(
                    &mut *tx,
                    count_id,
                    &actor.user_id,
                    now,
                    None,
                    &updated_totals,
                )
                .await?;

            let mut verified_count = count;
            verified_count.verified_by = Some(actor.user_id.clone());
            verified_count.verified_at = Some(now);
            return Ok(CountVerified::Verified(Box::new(verified_count)));
        }

        // For other count kinds:
        let totals = self.repo.get_count_totals(&mut *tx, count_id).await?;
        for t in &totals {
            CurrencyCode::parse(t.currency.trim()).map_err(|_| {
                AppError::Internal(format!("unknown currency code in count: {}", t.currency))
            })?;
        }
        self.repo
            .update_count_verified(&mut *tx, count_id, &actor.user_id, now, None, &totals)
            .await?;

        let mut verified_count = count;
        verified_count.verified_by = Some(actor.user_id.clone());
        verified_count.verified_at = Some(now);
        Ok(CountVerified::Verified(Box::new(verified_count)))
    }
}

/// The amount `held` (a drawer's ledger balances) has of `currency`, if it holds any.
pub(crate) fn held_of(held: &[(String, Decimal)], currency: &str) -> Option<Decimal> {
    held.iter()
        .find(|(code, _)| code.eq_ignore_ascii_case(currency.trim()))
        .map(|(_, amount)| *amount)
}

/// Opens `DRAWER(shift)`: one `OPENING` posting named by `source_id`, moving each currency's amount
/// from `from` into the drawer, one entry pair per currency (§4.4 rows 1 and 2; #929 option A). The OPEN
/// count, the hand-over and `adminOpenShift` all open a drawer here. A `from` that cannot cover it is
/// refused `stockInsufficient` by the port.
pub async fn post_opening_in(
    postings: &dyn CashPostings,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    from: CashPlace,
    shift_id: &str,
    amounts: &[(CurrencyCode, Decimal)],
    source_id: &str,
) -> AppResult<()> {
    let source = SourceRef {
        source_type: SourceType::Opening,
        source_id: source_id.to_owned(),
        source_key: "open".to_string(),
    };
    let reqs = moves(
        &from,
        &CashPlace::Drawer(shift_id.to_owned()),
        amounts,
        &source,
    );
    postings.post_in(tx, &reqs, None).await?;
    Ok(())
}

/// A drawer's count difference, (currency, counted − expected): one `COUNT` posting named by
/// `source_id`, a shortfall `DRAWER → VARIANCE(branch)` and an overage `VARIANCE → DRAWER` (§4.4 row 7).
/// The verified CLOSE count and `adminCloseShift` both post it here.
pub async fn post_difference_in(
    postings: &dyn CashPostings,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    shift_id: &str,
    branch: &BranchId,
    diffs: &[(CurrencyCode, Decimal)],
    source_id: &str,
) -> AppResult<()> {
    let source = SourceRef {
        source_type: SourceType::Count,
        source_id: source_id.to_owned(),
        source_key: "difference".to_string(),
    };
    let reqs = differences(
        &CashPlace::Drawer(shift_id.to_owned()),
        &CashPlace::Variance(branch.clone()),
        diffs,
        &source,
    );
    postings.post_in(tx, &reqs, None).await?;
    Ok(())
}
