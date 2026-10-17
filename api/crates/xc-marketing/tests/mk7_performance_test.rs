//! MK-7 #1406 — `PerformanceReader` (M04's figures, D-15), `PromoExpenseFacts` (executive's REWARD expense) and
//! `attributed_contribution` (design `docs/design/blueprint/marketing-campaigns.md` D-15, §6, §6.1; §9.1 row MK-7;
//! ticket #1406 §4 item 1, §7, §9; the lane controller claude/ad05bf8d's rulings 2–4 on #1406).
//! Written before the implementation by the test writer (claude/29df2ec5, the ticket author, rule A). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Locked here:
//! - REWARD expense = Σ REDEEMED REWARD-class redemptions in scope and period; PRICE never; REVERSED never; none ⇒ a
//!   real `0.00` (ruling 3).
//! - per version: redemptions, PRICE cost, REWARD expense, THB volume (each redeeming exchange once) and realized
//!   margin of the redeeming exchanges; REVERSED adds nothing (ruling 2).
//! - attributed contribution = realized margin − REWARD expense − commission of the redeeming exchanges; PRICE cost is
//!   never subtracted (D-15); `AwaitsLedger` when a redeeming line has no `realized_margin_thb`, else `AwaitsBroker`
//!   while commission is unavailable; never zero (ruling 4).
//!
//! Each database test works in a schema of its own, built from every migration in `api/migrations/`. Needs
//! `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips. Redemptions in the
//! period are one day old and the period is today−3 ..= today+1, so no branch timezone moves them across a bound;
//! redemptions outside it are twenty days old.

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{NaiveDate, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Money, MoneyDelta};
use xc_marketing::domain::performance::commission::{BrokerNotConnected, CampaignCommission};
use xc_marketing::domain::performance::contribution::attributed_contribution;
use xc_marketing::domain::performance::period::{PerformancePeriod, PerformanceScope};
use xc_marketing::domain::performance::promo_expense::PromoExpenseFacts;
use xc_marketing::domain::performance::unavailable::Unavailable;
use xc_marketing::domain::performance::version_performance::VersionPerformance;
use xc_marketing::infrastructure::pg_performance::PerformanceReader;

/// The organisation OR-1's migration creates.
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const MUST_FINISH: Duration = Duration::from_secs(60);

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

fn money(s: &str) -> Money {
    Money::parse(s).expect("a money amount")
}

/// A signed amount from text such as `"-7.00"` or `"53.00"`.
fn delta(s: &str) -> MoneyDelta {
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, s),
    };
    MoneyDelta::from_minor(sign * i128::from(money(digits).minor()))
}

/// today − 3 ..= today + 1 (UTC dates): a redemption one day old is inside in any timezone.
fn period() -> PerformancePeriod {
    let today = Utc::now().date_naive();
    PerformancePeriod::new(
        today - chrono::Duration::days(3),
        today + chrono::Duration::days(1),
    )
}

/// A period with nothing in it: far in the past.
fn empty_period() -> PerformancePeriod {
    PerformancePeriod::new(
        NaiveDate::from_ymd_opt(2001, 1, 1).expect("a date"),
        NaiveDate::from_ymd_opt(2001, 1, 31).expect("a date"),
    )
}

// ── fakes ───────────────────────────────────────────────────────────────────

/// A commission source answering 1.00 per distinct exchange it is asked about, and recording what it was asked.
#[derive(Default)]
struct OnePerExchange {
    asked: Mutex<Vec<BTreeSet<Uuid>>>,
}

impl CampaignCommission for OnePerExchange {
    fn commission_of<'a>(
        &'a self,
        transactions: &'a [Uuid],
    ) -> BoxFuture<'a, AppResult<Result<Money, Unavailable>>> {
        let set: BTreeSet<Uuid> = transactions.iter().copied().collect();
        let amount = Money::from_minor(100 * set.len() as i64).expect("non-negative");
        self.asked.lock().expect("lock").push(set);
        Box::pin(async move { Ok(Ok(amount)) })
    }
}

// ── the database ────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
    schema: String,
    branch: String,
    user: Uuid,
    customer: String,
}

/// One campaign version and one rule of each class.
#[derive(Clone, Copy)]
struct Version {
    version: Uuid,
    price: Uuid,
    reward: Uuid,
}

impl Db {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. The MK-7 tests need a real PostgreSQL and do not skip; point it at \
                 an existing database whose name ends in `_test`"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("mk7");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
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
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
            .run(&pool)
            .await
            .unwrap_or_else(|e| panic!("every migration applies on a fresh database: {e}"));
        let mut db = Self {
            pool,
            schema,
            branch: String::new(),
            user: Uuid::nil(),
            customer: unique("CX"),
        };
        db.branch = db.add_branch().await;
        db.user = db.add_user().await;
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2::uuid, $1, 'Somchai', 'LIVE', now())",
        )
        .bind(&db.customer)
        .bind(ORG_1)
        .execute(&db.pool)
        .await
        .expect("a customer");
        db
    }

    async fn finish(self) {
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.pool)
        .await
        .expect("drop the test schema");
        self.pool.close().await;
    }

    fn reader(&self, commission: Arc<dyn CampaignCommission>) -> PerformanceReader {
        PerformanceReader::new(self.pool.clone(), commission)
    }

    fn scope(&self) -> PerformanceScope {
        PerformanceScope::new(vec![BranchId::new(self.branch.as_str())])
    }

    async fn add_branch(&self) -> String {
        let branch = unique("B").replace('_', "-");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&branch)
            .bind(ORG_1)
            .execute(&self.pool)
            .await
            .expect("a branch");
        branch
    }

    async fn add_user(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, $3, 'ACTIVE', 'test')",
        )
        .bind(id)
        .bind(ORG_1)
        .bind(unique("user"))
        .execute(&self.pool)
        .await
        .expect("a user");
        id
    }

    /// An APPROVED, ACTIVE version of a new campaign with one PRICE rule (no. 1) and one REWARD rule (no. 2).
    async fn version(&self) -> Version {
        let campaign = Uuid::new_v4();
        let version = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, $4, now())",
        )
        .bind(campaign)
        .bind(ORG_1)
        .bind(unique("campaign"))
        .bind(self.user)
        .execute(&self.pool)
        .await
        .expect("a campaign");
        sqlx::query(
            "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, 1, 'APPROVED', date_trunc('minute', now()) - interval '60 days', \
             date_trunc('minute', now()) + interval '60 days', 100000, 'g', 1, 'hash', now(), $4, now())",
        )
        .bind(version)
        .bind(ORG_1)
        .bind(campaign)
        .bind(self.user)
        .execute(&self.pool)
        .await
        .expect("an APPROVED version");
        sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
            .bind(version)
            .bind(campaign)
            .execute(&self.pool)
            .await
            .expect("the current version");
        sqlx::query("INSERT INTO campaign_budgets (campaign_version_id) VALUES ($1)")
            .bind(version)
            .execute(&self.pool)
            .await
            .expect("the version's budget row");
        let mut rules = Vec::new();
        for (no, (kind, class)) in [("FEE_WAIVER", "PRICE"), ("REWARD_THB", "REWARD")]
            .into_iter()
            .enumerate()
        {
            let rule = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
                 calculation, caps, requires_coupon, rule_no) \
                 VALUES ($1, $2, $3, $4, '{}'::jsonb, '{}'::jsonb, '{}'::jsonb, false, $5)",
            )
            .bind(rule)
            .bind(version)
            .bind(kind)
            .bind(class)
            .bind(no as i32 + 1)
            .execute(&self.pool)
            .await
            .expect("a rule");
            rules.push(rule);
        }
        Version {
            version,
            price: rules[0],
            reward: rules[1],
        }
    }

    /// A COMPLETED LIVE exchange in `branch` whose lines are `(thb_amount, realized_margin_thb)`; `None` = the ledger
    /// has not posted that line's margin.
    async fn exchange(&self, branch: &str, lines: &[(&str, Option<&str>)]) -> Uuid {
        let teller = self.add_user().await;
        let shift = unique("shift");
        sqlx::query(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE branch_id = $1 AND status = 'OPEN'",
        )
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("close the branch's open shift");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
        )
        .bind(&shift)
        .bind(branch)
        .bind(unique("open-req"))
        .bind(teller)
        .execute(&self.pool)
        .await
        .expect("a shift");
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
             customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
             recorded_at, recorded_by, status, completed_at) \
             VALUES ($1, $2::uuid, $3, $4, 'LIVE', $6, $5, 'hash', $7, 'Somchai', 'THA', 'system', 'WALK-IN', \
             '', current_date, now(), now(), 'system', 'COMPLETED', now())",
        )
        .bind(id)
        .bind(ORG_1)
        .bind(unique("DOC"))
        .bind(branch)
        .bind(unique("sub"))
        .bind(&shift)
        .bind(&self.customer)
        .execute(&self.pool)
        .await
        .expect("a completed transaction");
        for (no, (thb, margin)) in lines.iter().enumerate() {
            sqlx::query(
                "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, quantity, \
                 unit_size, rate_snapshot, thb_amount, settle_thb, realized_margin_thb, line_no, created_at, \
                 updated_at) \
                 VALUES ($1, $2::uuid, $3, 'SELL', 'USD', 100, 1, 32, $4, $4, $5, $6, now(), now())",
            )
            .bind(Uuid::new_v4())
            .bind(ORG_1)
            .bind(id)
            .bind(d(thb))
            .bind(margin.map(d))
            .bind(no as i32 + 1)
            .execute(&self.pool)
            .await
            .expect("a transaction line");
        }
        id
    }

    /// A quote of `branch` (its reservation is CONSUMED, so it holds no budget).
    async fn quote(&self, branch: &str) -> Uuid {
        let request = Uuid::new_v4();
        let publication = Uuid::new_v4();
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, \
             policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES ($1, $2::uuid, 'RATE_PUBLICATION', $3, $4::text, 'test', '1', '{}'::jsonb, 'test', now(), \
             'APPROVED', $5)",
        )
        .bind(request)
        .bind(ORG_1)
        .bind(branch)
        .bind(publication.to_string())
        .bind(unique("pub"))
        .execute(&self.pool)
        .await
        .expect("an approved publication request");
        sqlx::query(
            "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
             approved_request_id, published_by) \
             SELECT $1, $2::uuid, $3, COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = $3), 0) + 1, \
             now(), $4, 'test'",
        )
        .bind(publication)
        .bind(ORG_1)
        .bind(branch)
        .bind(request)
        .execute(&self.pool)
        .await
        .expect("a publication");
        sqlx::query(
            "INSERT INTO quotes (id, organization_id, branch_id, customer_id, type, channel, status, publication_id, \
             expires_at, created_by, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, NULL, 'INDICATIVE', 'DESK', 'ACTIVE', $4, now() + interval '15 minutes', \
             'test', now(), now())",
        )
        .bind(id)
        .bind(ORG_1)
        .bind(branch)
        .bind(publication)
        .execute(&self.pool)
        .await
        .expect("a quote");
        id
    }

    /// A redemption of `v`'s rule of `class` by `transaction` for `amount`, `days_ago` days old (minute-truncated);
    /// `reversed` marks it REVERSED with a reversal row. Its quote is in the exchange's branch; its reservation is
    /// CONSUMED.
    async fn redeem(
        &self,
        v: Version,
        class: &str,
        transaction: Uuid,
        amount: &str,
        days_ago: i64,
        reversed: bool,
    ) {
        let rule = if class == "PRICE" { v.price } else { v.reward };
        let branch: String = sqlx::query_scalar("SELECT branch_id FROM transactions WHERE id = $1")
            .bind(transaction)
            .fetch_one(&self.pool)
            .await
            .expect("the exchange's branch");
        let branch = branch.as_str();
        let quote = self.quote(branch).await;
        let reservation = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO benefit_reservations (id, organization_id, rule_id, campaign_version_id, quote_id, \
             amount_thb, status, expires_at, created_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, 'CONSUMED', now() + interval '15 minutes', now())",
        )
        .bind(reservation)
        .bind(ORG_1)
        .bind(rule)
        .bind(v.version)
        .bind(quote)
        .bind(d(amount))
        .execute(&self.pool)
        .await
        .expect("a consumed reservation");
        let reversal = if reversed {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, reason, \
                 amount_thb, business_date, requested_by, requested_at) \
                 VALUES ($1, $2::uuid, $3, $4, 'REQUESTED', 'test', 3200, current_date, 'test', now())",
            )
            .bind(id)
            .bind(ORG_1)
            .bind(branch)
            .bind(transaction)
            .execute(&self.pool)
            .await
            .expect("a reversal");
            Some(id)
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO benefit_redemptions (id, organization_id, rule_id, campaign_version_id, reservation_id, \
             transaction_id, benefit_class, amount_thb, status, reversal_id, redeemed_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, $9, $10, \
             date_trunc('minute', now()) - make_interval(days => $11))",
        )
        .bind(Uuid::new_v4())
        .bind(ORG_1)
        .bind(rule)
        .bind(v.version)
        .bind(reservation)
        .bind(transaction)
        .bind(class)
        .bind(d(amount))
        .bind(if reversed { "REVERSED" } else { "REDEEMED" })
        .bind(reversal)
        .bind(days_ago as i32)
        .execute(&self.pool)
        .await
        .expect("a redemption");
    }
}

impl Db {
    /// [`Db::redeem`] REDEEMED at the explicit instant `at` (RFC 3339 with offset, e.g. `2026-01-10T00:05:00+07:00`).
    async fn redeem_at(&self, v: Version, class: &str, transaction: Uuid, amount: &str, at: &str) {
        self.redeem(v, class, transaction, amount, 0, false).await;
        let at = chrono::DateTime::parse_from_rfc3339(at).expect("an RFC 3339 instant");
        let rule = if class == "PRICE" { v.price } else { v.reward };
        sqlx::query(
            "UPDATE benefit_redemptions SET redeemed_at = $1 WHERE rule_id = $2 AND transaction_id = $3",
        )
        .bind(at.with_timezone(&Utc))
        .bind(rule)
        .bind(transaction)
        .execute(&self.pool)
        .await
        .expect("move the redemption to its instant");
    }

    /// The bounds fixture: the room's branch on `Asia/Bangkok`, one version, four REWARD redemptions around the
    /// fixed period 2026-01-10 ..= 2026-01-12 — inside at 01-10 00:05+07 (1.00, still 01-09 in UTC) and 01-12
    /// 23:55+07 (2.00); outside at 01-09 23:55+07 (4.00) and 01-13 00:05+07 (8.00, still 01-12 in UTC).
    async fn bounds_fixture(&self) -> Version {
        sqlx::query("UPDATE branches SET timezone = 'Asia/Bangkok' WHERE id = $1")
            .bind(&self.branch)
            .execute(&self.pool)
            .await
            .expect("a Bangkok branch");
        let v = self.version().await;
        for (amount, at) in [
            ("1.00", "2026-01-10T00:05:00+07:00"),
            ("2.00", "2026-01-12T23:55:00+07:00"),
            ("4.00", "2026-01-09T23:55:00+07:00"),
            ("8.00", "2026-01-13T00:05:00+07:00"),
        ] {
            let x = self
                .exchange(&self.branch, &[("100.00", Some("1.00"))])
                .await;
            self.redeem_at(v, "REWARD", x, amount, at).await;
        }
        v
    }
}

/// 2026-01-10 ..= 2026-01-12, fixed: independent of the clock.
fn fixed_period() -> PerformancePeriod {
    PerformancePeriod::new(
        NaiveDate::from_ymd_opt(2026, 1, 10).expect("a date"),
        NaiveDate::from_ymd_opt(2026, 1, 12).expect("a date"),
    )
}

fn row_of(rows: &[VersionPerformance], v: Version) -> &VersionPerformance {
    rows.iter()
        .find(|r| r.version_id == v.version)
        .unwrap_or_else(|| panic!("a row for version {}: {rows:?}", v.version))
}

// ── PromoExpenseFacts::reward_expense (ruling 3) ────────────────────────────

#[tokio::test]
async fn reward_expense_sums_the_redeemed_reward_redemptions_in_scope_and_period() {
    let db = Db::start().await;
    let a = db.version().await;
    let b = db.version().await;
    let x1 = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    let x2 = db.exchange(&db.branch, &[("1000.00", Some("10.00"))]).await;
    db.redeem(a, "REWARD", x1, "30.00", 1, false).await;
    db.redeem(b, "REWARD", x2, "12.50", 1, false).await;

    let got = db
        .reader(Arc::new(BrokerNotConnected))
        .reward_expense(&db.scope(), &period())
        .await
        .expect("reward_expense answers");
    assert_eq!(
        got,
        Ok(money("42.50")),
        "Σ of every REDEEMED REWARD redemption of the scope in the period, across versions"
    );
    db.finish().await;
}

#[tokio::test]
async fn reward_expense_never_includes_price_cost() {
    let db = Db::start().await;
    let v = db.version().await;
    let x = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    db.redeem(v, "PRICE", x, "100.00", 1, false).await;
    db.redeem(v, "REWARD", x, "5.00", 1, false).await;

    let got = db
        .reader(Arc::new(BrokerNotConnected))
        .reward_expense(&db.scope(), &period())
        .await
        .expect("reward_expense answers");
    assert_eq!(
        got,
        Ok(money("5.00")),
        "PRICE cost is already inside the realized margin and is never promo/reward expense (D-15)"
    );
    db.finish().await;
}

#[tokio::test]
async fn reward_expense_excludes_reversed_redemptions() {
    let db = Db::start().await;
    let v = db.version().await;
    let x1 = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    let x2 = db.exchange(&db.branch, &[("1000.00", Some("10.00"))]).await;
    db.redeem(v, "REWARD", x1, "40.00", 1, true).await;
    db.redeem(v, "REWARD", x2, "7.00", 1, false).await;

    let got = db
        .reader(Arc::new(BrokerNotConnected))
        .reward_expense(&db.scope(), &period())
        .await
        .expect("reward_expense answers");
    assert_eq!(
        got,
        Ok(money("7.00")),
        "a REVERSED redemption's budget was returned (M3); it adds nothing (ruling 2)"
    );
    db.finish().await;
}

#[tokio::test]
async fn reward_expense_with_no_redemption_is_a_real_zero() {
    let db = Db::start().await;
    let v = db.version().await;
    let x = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    db.redeem(v, "REWARD", x, "9.00", 1, false).await;

    let got = db
        .reader(Arc::new(BrokerNotConnected))
        .reward_expense(&db.scope(), &empty_period())
        .await
        .expect("reward_expense answers");
    assert_eq!(
        got,
        Ok(Money::ZERO),
        "no redemption in the period is a real 0.00, never unavailable (ruling 3)"
    );
    db.finish().await;
}

#[tokio::test]
async fn reward_expense_excludes_other_branches_and_other_periods() {
    let db = Db::start().await;
    let v = db.version().await;
    let elsewhere = db.add_branch().await;
    let x_in = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    let x_old = db.exchange(&db.branch, &[("1000.00", Some("10.00"))]).await;
    let x_other = db.exchange(&elsewhere, &[("500.00", Some("5.00"))]).await;
    db.redeem(v, "REWARD", x_in, "9.00", 1, false).await;
    db.redeem(v, "REWARD", x_old, "60.00", 20, false).await;
    db.redeem(v, "REWARD", x_other, "50.00", 1, false).await;

    let got = db
        .reader(Arc::new(BrokerNotConnected))
        .reward_expense(&db.scope(), &period())
        .await
        .expect("reward_expense answers");
    assert_eq!(
        got,
        Ok(money("9.00")),
        "only redemptions whose exchange is in a scope branch and whose redeemed_at is in the period"
    );
    db.finish().await;
}

// ── PerformanceReader::by_version (D-15, rulings 2 and 4) ───────────────────

#[tokio::test]
async fn performance_counts_each_versions_redemptions_costs_volume_and_margin() {
    let db = Db::start().await;
    let v1 = db.version().await;
    let v2 = db.version().await;
    // x1 redeems both of v1's rules: its volume and margin count once.
    let x1 = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    let x2 = db
        .exchange(
            &db.branch,
            &[("1000.00", Some("15.50")), ("500.00", Some("4.50"))],
        )
        .await;
    let x3 = db.exchange(&db.branch, &[("100.00", Some("1.00"))]).await;
    db.redeem(v1, "PRICE", x1, "10.00", 1, false).await;
    db.redeem(v1, "REWARD", x1, "5.00", 1, false).await;
    db.redeem(v1, "PRICE", x2, "3.00", 1, false).await;
    db.redeem(v2, "REWARD", x3, "1.00", 1, false).await;

    let rows = db
        .reader(Arc::new(OnePerExchange::default()))
        .by_version(&db.scope(), &period())
        .await
        .expect("by_version answers");
    let r1 = row_of(&rows, v1);
    assert_eq!(r1.redemptions, 3, "{r1:?}");
    assert_eq!(r1.price_cost, money("13.00"), "{r1:?}");
    assert_eq!(r1.reward_expense, money("5.00"), "{r1:?}");
    assert_eq!(
        r1.volume_thb,
        money("4700.00"),
        "each redeeming exchange's lines once: 3200 + 1000 + 500 ({r1:?})"
    );
    assert_eq!(r1.realized_margin, Ok(delta("60.00")), "{r1:?}");
    let r2 = row_of(&rows, v2);
    assert_eq!(r2.redemptions, 1, "{r2:?}");
    assert_eq!(r2.price_cost, Money::ZERO, "{r2:?}");
    assert_eq!(r2.reward_expense, money("1.00"), "{r2:?}");
    assert_eq!(r2.volume_thb, money("100.00"), "{r2:?}");
    assert_eq!(r2.realized_margin, Ok(delta("1.00")), "{r2:?}");
    db.finish().await;
}

#[tokio::test]
async fn contribution_is_margin_minus_reward_minus_the_commission_of_the_versions_exchanges() {
    let db = Db::start().await;
    let v = db.version().await;
    let x1 = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    let x2 = db.exchange(&db.branch, &[("1500.00", Some("20.00"))]).await;
    db.redeem(v, "PRICE", x1, "10.00", 1, false).await;
    db.redeem(v, "REWARD", x1, "5.00", 1, false).await;
    db.redeem(v, "PRICE", x2, "3.00", 1, false).await;
    // a second version with an exchange of its own: its commission is never v's
    let other = db.version().await;
    let x3 = db.exchange(&db.branch, &[("700.00", Some("9.00"))]).await;
    db.redeem(other, "REWARD", x3, "1.00", 1, false).await;
    let commission = Arc::new(OnePerExchange::default());

    let rows = db
        .reader(commission.clone())
        .by_version(&db.scope(), &period())
        .await
        .expect("by_version answers");
    let r = row_of(&rows, v);
    assert_eq!(
        r.contribution,
        Ok(delta("53.00")),
        "60.00 margin − 5.00 REWARD − 2.00 commission; the 13.00 PRICE cost is never subtracted ({r:?})"
    );
    let asked = commission.asked.lock().expect("lock").clone();
    assert!(
        asked.contains(&BTreeSet::from([x1, x2])),
        "commission is asked for the version's redeeming exchanges: {asked:?}"
    );
    let mut sets = asked.clone();
    sets.sort();
    let mut want = vec![BTreeSet::from([x1, x2]), BTreeSet::from([x3])];
    want.sort();
    assert_eq!(
        sets, want,
        "one ask per version, each exactly that version's own redeeming exchanges, never another version's"
    );
    let o = row_of(&rows, other);
    assert_eq!(
        o.contribution,
        Ok(delta("7.00")),
        "9.00 margin − 1.00 REWARD − 1.00 commission of its own exchange ({o:?})"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_price_only_redemption_gives_contribution_of_margin_minus_commission() {
    let db = Db::start().await;
    let v = db.version().await;
    let x = db.exchange(&db.branch, &[("2000.00", Some("25.00"))]).await;
    db.redeem(v, "PRICE", x, "8.00", 1, false).await;

    let rows = db
        .reader(Arc::new(OnePerExchange::default()))
        .by_version(&db.scope(), &period())
        .await
        .expect("by_version answers");
    let r = row_of(&rows, v);
    assert_eq!(r.price_cost, money("8.00"), "{r:?}");
    assert_eq!(r.reward_expense, Money::ZERO, "{r:?}");
    assert_eq!(
        r.contribution,
        Ok(delta("24.00")),
        "25.00 margin − 1.00 commission; the 8.00 PRICE cost is inside the margin already (§9 edge 2) ({r:?})"
    );
    db.finish().await;
}

#[tokio::test]
async fn contribution_awaits_the_ledger_when_a_redeeming_line_has_no_realized_margin() {
    let db = Db::start().await;
    let v = db.version().await;
    let x1 = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    let x2 = db
        .exchange(&db.branch, &[("1000.00", Some("10.00")), ("400.00", None)])
        .await;
    db.redeem(v, "REWARD", x1, "5.00", 1, false).await;
    db.redeem(v, "PRICE", x2, "2.00", 1, false).await;

    let rows = db
        .reader(Arc::new(OnePerExchange::default()))
        .by_version(&db.scope(), &period())
        .await
        .expect("by_version answers");
    let r = row_of(&rows, v);
    assert_eq!(
        r.realized_margin,
        Err(Unavailable::AwaitsLedger),
        "a NULL realized_margin_thb is not zero ({r:?})"
    );
    assert_eq!(
        r.contribution,
        Err(Unavailable::AwaitsLedger),
        "unavailable, never a number, even with commission available (ruling 4) ({r:?})"
    );
    assert_eq!(r.redemptions, 2, "the other figures still show ({r:?})");
    assert_eq!(r.price_cost, money("2.00"), "{r:?}");
    assert_eq!(r.reward_expense, money("5.00"), "{r:?}");
    assert_eq!(r.volume_thb, money("4600.00"), "{r:?}");
    db.finish().await;
}

#[tokio::test]
async fn contribution_awaits_broker_while_commission_is_unavailable() {
    let db = Db::start().await;
    let v = db.version().await;
    let x = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    db.redeem(v, "REWARD", x, "5.00", 1, false).await;

    let rows = db
        .reader(Arc::new(BrokerNotConnected))
        .by_version(&db.scope(), &period())
        .await
        .expect("by_version answers");
    let r = row_of(&rows, v);
    assert_eq!(r.realized_margin, Ok(delta("40.00")), "{r:?}");
    assert_eq!(
        r.contribution,
        Err(Unavailable::AwaitsBroker),
        "commission is unavailable today (EX-10 parked): contribution is unavailable, never 35.00 (ruling 4) ({r:?})"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_reversed_redemption_adds_nothing_to_its_version() {
    let db = Db::start().await;
    let v = db.version().await;
    let x1 = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    // x2's margin is not posted: were the REVERSED redemption counted, margin and contribution would await the ledger.
    let x2 = db.exchange(&db.branch, &[("900.00", None)]).await;
    db.redeem(v, "PRICE", x1, "10.00", 1, false).await;
    db.redeem(v, "REWARD", x2, "6.00", 1, true).await;

    let rows = db
        .reader(Arc::new(OnePerExchange::default()))
        .by_version(&db.scope(), &period())
        .await
        .expect("by_version answers");
    let r = row_of(&rows, v);
    assert_eq!(r.redemptions, 1, "{r:?}");
    assert_eq!(r.price_cost, money("10.00"), "{r:?}");
    assert_eq!(r.reward_expense, Money::ZERO, "{r:?}");
    assert_eq!(r.volume_thb, money("3200.00"), "{r:?}");
    assert_eq!(r.realized_margin, Ok(delta("40.00")), "{r:?}");
    assert_eq!(r.contribution, Ok(delta("39.00")), "{r:?}");
    db.finish().await;
}

#[tokio::test]
async fn performance_excludes_other_branches_and_other_periods() {
    let db = Db::start().await;
    let v = db.version().await;
    let only_outside = db.version().await;
    let elsewhere = db.add_branch().await;
    let x_in = db.exchange(&db.branch, &[("3200.00", Some("40.00"))]).await;
    let x_old = db.exchange(&db.branch, &[("1000.00", Some("10.00"))]).await;
    let x_other = db.exchange(&elsewhere, &[("500.00", Some("5.00"))]).await;
    let x_other2 = db.exchange(&elsewhere, &[("700.00", Some("7.00"))]).await;
    db.redeem(v, "PRICE", x_in, "4.00", 1, false).await;
    db.redeem(v, "PRICE", x_old, "60.00", 20, false).await;
    db.redeem(v, "REWARD", x_other, "50.00", 1, false).await;
    db.redeem(only_outside, "PRICE", x_other2, "3.00", 1, false)
        .await;

    let rows = db
        .reader(Arc::new(OnePerExchange::default()))
        .by_version(&db.scope(), &period())
        .await
        .expect("by_version answers");
    let r = row_of(&rows, v);
    assert_eq!(r.redemptions, 1, "{r:?}");
    assert_eq!(r.price_cost, money("4.00"), "{r:?}");
    assert_eq!(r.reward_expense, Money::ZERO, "{r:?}");
    assert_eq!(r.volume_thb, money("3200.00"), "{r:?}");
    assert_eq!(r.realized_margin, Ok(delta("40.00")), "{r:?}");
    if let Some(o) = rows.iter().find(|r| r.version_id == only_outside.version) {
        assert_eq!(
            (o.redemptions, o.price_cost, o.volume_thb),
            (0, Money::ZERO, Money::ZERO),
            "a version redeemed only outside the scope counts nothing: {o:?}"
        );
    }
    db.finish().await;
}

// ── period bounds and timezone (review must-fix 1) ──────────────────────────

#[tokio::test]
async fn reward_expense_period_is_inclusive_business_days_in_the_branch_timezone() {
    let db = Db::start().await;
    db.bounds_fixture().await;

    let got = db
        .reader(Arc::new(BrokerNotConnected))
        .reward_expense(&db.scope(), &fixed_period())
        .await
        .expect("reward_expense answers");
    assert_eq!(
        got,
        Ok(money("3.00")),
        "exactly 01-10 00:05+07 (1.00) and 01-12 23:55+07 (2.00): both bounds inclusive, dates in the branch's \
         timezone, never UTC; 01-09 23:55+07 (4.00) and 01-13 00:05+07 (8.00) are outside"
    );
    db.finish().await;
}

#[tokio::test]
async fn performance_period_is_inclusive_business_days_in_the_branch_timezone() {
    let db = Db::start().await;
    let v = db.bounds_fixture().await;

    let rows = db
        .reader(Arc::new(OnePerExchange::default()))
        .by_version(&db.scope(), &fixed_period())
        .await
        .expect("by_version answers");
    let r = row_of(&rows, v);
    assert_eq!(
        r.redemptions, 2,
        "exactly the two redemptions inside 2026-01-10 ..= 2026-01-12, Asia/Bangkok ({r:?})"
    );
    assert_eq!(r.reward_expense, money("3.00"), "{r:?}");
    assert_eq!(r.volume_thb, money("200.00"), "{r:?}");
    db.finish().await;
}

// ── attributed_contribution (pure; D-15, ruling 4) ─────────────────────────

#[test]
fn attributed_contribution_is_margin_minus_reward_minus_commission() {
    assert_eq!(
        attributed_contribution(Ok(delta("100.00")), money("10.00"), Ok(money("5.25"))),
        Ok(delta("84.75"))
    );
}

#[test]
fn attributed_contribution_may_be_negative() {
    assert_eq!(
        attributed_contribution(Ok(delta("3.00")), money("10.00"), Ok(Money::ZERO)),
        Ok(delta("-7.00"))
    );
    assert_eq!(
        attributed_contribution(Ok(delta("-2.00")), Money::ZERO, Ok(money("1.00"))),
        Ok(delta("-3.00"))
    );
}

#[test]
fn an_unavailable_margin_makes_contribution_await_the_ledger_whatever_the_commission() {
    assert_eq!(
        attributed_contribution(
            Err(Unavailable::AwaitsLedger),
            money("1.00"),
            Err(Unavailable::AwaitsBroker)
        ),
        Err(Unavailable::AwaitsLedger)
    );
    assert_eq!(
        attributed_contribution(
            Err(Unavailable::AwaitsLedger),
            Money::ZERO,
            Ok(money("2.00"))
        ),
        Err(Unavailable::AwaitsLedger)
    );
}

#[test]
fn an_unavailable_commission_makes_contribution_unavailable_never_zero() {
    assert_eq!(
        attributed_contribution(
            Ok(delta("50.00")),
            money("4.00"),
            Err(Unavailable::AwaitsBroker)
        ),
        Err(Unavailable::AwaitsBroker)
    );
}
