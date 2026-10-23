//! `PgLedgerShiftBalances`, the PostgreSQL `ShiftBalances` over the ledger (design
//! `docs/design/blueprint/ledger-inventory.md` §4.2, §4.2.1).

use std::collections::BTreeMap;

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{FromRow, PgPool};
use xc_kernel::money::{CurrencyCode, ForeignAmount, QuantityDelta};
use xc_shift::domain::shift::repositories::shift_balances::{ShiftBalance, ShiftBalances, ShiftId};

/// `ShiftBalances` read from `cash_accounts`, `ledger_postings`, `ledger_entries`, `cash_counts` and
/// `cash_count_totals`.
pub struct PgLedgerShiftBalances {
    pool: PgPool,
}

impl PgLedgerShiftBalances {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn to_money(v: Decimal, currency: CurrencyCode, max_scale: u32) -> AppResult<ForeignAmount> {
    let mut d = v.normalize();
    if d.scale() > max_scale {
        return Err(AppError::Internal(format!(
            "ledger balance for {currency} has scale {} > minor_unit {max_scale}: {v}",
            d.scale()
        )));
    }
    if d.scale() < 2 {
        d.rescale(2);
    }
    ForeignAmount::checked_from_decimal(d).map_err(|_| {
        AppError::Internal(format!(
            "ledger balance for {currency} is not a money amount: {v}"
        ))
    })
}

fn to_delta(v: Decimal, currency: CurrencyCode, max_scale: u32) -> AppResult<QuantityDelta> {
    let mut d = v.normalize();
    if d.scale() > max_scale {
        return Err(AppError::Internal(format!(
            "ledger balance for {currency} has scale {} > minor_unit {max_scale}: {v}",
            d.scale()
        )));
    }
    if d.scale() < 2 {
        d.rescale(2);
    }
    QuantityDelta::from_decimal(d)
        .map_err(|_| AppError::Internal(format!("ledger balance for {currency} out of range: {v}")))
}

#[derive(FromRow)]
struct ShiftBalanceRow {
    currency: String,
    minor_unit: Option<i16>,
    opening: Decimal,
    expected: Decimal,
    moved_in: Decimal,
    moved_out: Decimal,
    counted: Option<Decimal>,
    difference: Option<Decimal>,
}

#[derive(FromRow)]
struct ShiftBalanceManyRow {
    shift_id: String,
    currency: String,
    minor_unit: Option<i16>,
    opening: Decimal,
    expected: Decimal,
    moved_in: Decimal,
    moved_out: Decimal,
    counted: Option<Decimal>,
    difference: Option<Decimal>,
}

#[allow(clippy::too_many_arguments)]
fn into_shift_balance(
    currency: &str,
    minor_unit: Option<i16>,
    opening: Decimal,
    expected: Decimal,
    moved_in: Decimal,
    moved_out: Decimal,
    counted: Option<Decimal>,
    difference: Option<Decimal>,
) -> AppResult<ShiftBalance> {
    let currency = CurrencyCode::parse(currency)
        .map_err(|_| AppError::Internal(format!("invalid currency code: {currency}")))?;
    let max_scale = minor_unit.unwrap_or(2).max(0) as u32;
    let opening = to_money(opening, currency, max_scale)?;
    let expected = to_delta(expected, currency, max_scale)?;
    let moved_in = to_money(moved_in, currency, max_scale)?;
    let moved_out = to_money(moved_out, currency, max_scale)?;
    let counted = counted
        .map(|v| to_money(v, currency, max_scale))
        .transpose()?;
    let difference = difference
        .map(|v| to_delta(v, currency, max_scale))
        .transpose()?;

    Ok(ShiftBalance {
        currency,
        opening,
        expected,
        moved_in,
        moved_out,
        counted,
        difference,
    })
}

const SHIFT_BALANCES_SQL: &str = r#"
WITH drawer_balances AS (
    SELECT
        a.currency,
        coalesce(sum(CASE WHEN p.source_type = 'OPENING' AND e.quantity_signed > 0 THEN e.quantity_signed ELSE 0 END), 0) AS opening,
        coalesce(sum(e.quantity_signed), 0) AS expected,
        coalesce(sum(CASE WHEN p.source_type NOT IN ('EXCHANGE', 'OPENING') AND e.quantity_signed > 0 THEN e.quantity_signed ELSE 0 END), 0) AS moved_in,
        coalesce(sum(CASE WHEN p.source_type NOT IN ('EXCHANGE', 'OPENING') AND e.quantity_signed < 0 THEN abs(e.quantity_signed) ELSE 0 END), 0) AS moved_out
    FROM cash_accounts a
    LEFT JOIN ledger_entries e ON e.account_id = a.id
    LEFT JOIN ledger_postings p ON p.id = e.posting_id
    WHERE a.kind = 'DRAWER' AND a.shift_id = $1
    GROUP BY a.currency
),
latest_count AS (
    SELECT id
    FROM cash_counts
    WHERE shift_id = $1
      AND verified_at IS NOT NULL
      AND kind IN ('CLOSE', 'HANDOVER_OUT')
    ORDER BY verified_at DESC, id DESC
    LIMIT 1
),
count_balances AS (
    SELECT
        cct.currency,
        cct.counted,
        cct.difference
    FROM latest_count lc
    JOIN cash_count_totals cct ON cct.count_id = lc.id
)
SELECT
    trim(coalesce(d.currency, c.currency))::text AS currency,
    cur.minor_unit AS minor_unit,
    coalesce(d.opening, 0) AS opening,
    coalesce(d.expected, 0) AS expected,
    coalesce(d.moved_in, 0) AS moved_in,
    coalesce(d.moved_out, 0) AS moved_out,
    c.counted,
    c.difference
FROM drawer_balances d
FULL OUTER JOIN count_balances c ON c.currency = d.currency
LEFT JOIN currencies cur ON cur.code = coalesce(d.currency, c.currency)
ORDER BY
    CASE WHEN coalesce(d.currency, c.currency) = 'THB' THEN 0 ELSE 1 END,
    coalesce(d.currency, c.currency) ASC
"#;

const SHIFT_BALANCES_MANY_SQL: &str = r#"
WITH drawer_balances AS (
    SELECT
        a.shift_id,
        a.currency,
        coalesce(sum(CASE WHEN p.source_type = 'OPENING' AND e.quantity_signed > 0 THEN e.quantity_signed ELSE 0 END), 0) AS opening,
        coalesce(sum(e.quantity_signed), 0) AS expected,
        coalesce(sum(CASE WHEN p.source_type NOT IN ('EXCHANGE', 'OPENING') AND e.quantity_signed > 0 THEN e.quantity_signed ELSE 0 END), 0) AS moved_in,
        coalesce(sum(CASE WHEN p.source_type NOT IN ('EXCHANGE', 'OPENING') AND e.quantity_signed < 0 THEN abs(e.quantity_signed) ELSE 0 END), 0) AS moved_out
    FROM cash_accounts a
    LEFT JOIN ledger_entries e ON e.account_id = a.id
    LEFT JOIN ledger_postings p ON p.id = e.posting_id
    WHERE a.kind = 'DRAWER' AND a.shift_id = ANY($1)
    GROUP BY a.shift_id, a.currency
),
latest_count AS (
    SELECT DISTINCT ON (shift_id)
        shift_id,
        id
    FROM cash_counts
    WHERE shift_id = ANY($1)
      AND verified_at IS NOT NULL
      AND kind IN ('CLOSE', 'HANDOVER_OUT')
    ORDER BY shift_id, verified_at DESC, id DESC
),
count_balances AS (
    SELECT
        lc.shift_id,
        cct.currency,
        cct.counted,
        cct.difference
    FROM latest_count lc
    JOIN cash_count_totals cct ON cct.count_id = lc.id
)
SELECT
    coalesce(d.shift_id, c.shift_id) AS shift_id,
    trim(coalesce(d.currency, c.currency))::text AS currency,
    cur.minor_unit AS minor_unit,
    coalesce(d.opening, 0) AS opening,
    coalesce(d.expected, 0) AS expected,
    coalesce(d.moved_in, 0) AS moved_in,
    coalesce(d.moved_out, 0) AS moved_out,
    c.counted,
    c.difference
FROM drawer_balances d
FULL OUTER JOIN count_balances c
    ON c.shift_id = d.shift_id AND c.currency = d.currency
LEFT JOIN currencies cur ON cur.code = coalesce(d.currency, c.currency)
ORDER BY
    coalesce(d.shift_id, c.shift_id) ASC,
    CASE WHEN coalesce(d.currency, c.currency) = 'THB' THEN 0 ELSE 1 END,
    coalesce(d.currency, c.currency) ASC
"#;

impl ShiftBalances for PgLedgerShiftBalances {
    fn shift_balances<'a>(
        &'a self,
        shift: &'a ShiftId,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftBalance>>> {
        Box::pin(async move {
            let rows: Vec<ShiftBalanceRow> = sqlx::query_as(SHIFT_BALANCES_SQL)
                .bind(shift)
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?;

            let mut balances = Vec::with_capacity(rows.len());
            for r in rows {
                balances.push(into_shift_balance(
                    &r.currency,
                    r.minor_unit,
                    r.opening,
                    r.expected,
                    r.moved_in,
                    r.moved_out,
                    r.counted,
                    r.difference,
                )?);
            }

            Ok(balances)
        })
    }

    fn shift_balances_many<'a>(
        &'a self,
        shifts: &'a [ShiftId],
    ) -> BoxFuture<'a, AppResult<BTreeMap<ShiftId, Vec<ShiftBalance>>>> {
        Box::pin(async move {
            let mut out: BTreeMap<ShiftId, Vec<ShiftBalance>> =
                shifts.iter().map(|s| (s.clone(), Vec::new())).collect();
            if out.is_empty() {
                return Ok(out);
            }

            let rows: Vec<ShiftBalanceManyRow> = sqlx::query_as(SHIFT_BALANCES_MANY_SQL)
                .bind(shifts)
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?;

            for r in rows {
                let balance = into_shift_balance(
                    &r.currency,
                    r.minor_unit,
                    r.opening,
                    r.expected,
                    r.moved_in,
                    r.moved_out,
                    r.counted,
                    r.difference,
                )?;
                if let Some(entry) = out.get_mut(&r.shift_id) {
                    entry.push(balance);
                }
            }

            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::AssertSqlSafe;
    use sqlx::migrate::Migrator;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use std::path::{Path, PathBuf};
    use std::str::FromStr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use uuid::Uuid;
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

    fn migrations_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
    }

    async fn db() -> (PgPool, BranchId) {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. Point it at an existing database ending in _test."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let schema = unique("lg6latest");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to test db");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read migrations")
            .run(&pool)
            .await
            .expect("run migrations");

        let branch = unique("B");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&branch)
            .execute(&pool)
            .await
            .expect("insert branch");
        sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
            .bind(&branch)
            .execute(&pool)
            .await
            .expect("insert ops_state");

        (pool, BranchId::new(branch))
    }

    async fn seed_user(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, 'Test Teller', 'ACTIVE', 'test')",
        )
        .bind(id)
        .bind(ORG)
        .execute(pool)
        .await
        .expect("insert user");
        id
    }

    async fn create_shift(pool: &PgPool, branch: &BranchId) -> String {
        let teller_id = seed_user(pool).await;
        let shift_id = unique("shift");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
        )
        .bind(&shift_id)
        .bind(branch.as_str())
        .bind(unique("open-req"))
        .bind(teller_id)
        .execute(pool)
        .await
        .expect("insert shift");
        shift_id
    }

    async fn seed_verified_close_count(
        pool: &PgPool,
        branch: &BranchId,
        shift: &str,
        verified_offset_secs: i64,
        total: (&str, &str, &str, &str),
    ) {
        let (currency, counted, expected, difference) = total;
        let counter = seed_user(pool).await;
        let verifier = seed_user(pool).await;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, \
             verified_by, verified_at, idempotency_key) \
             VALUES ($1::uuid, $2, $3, 'CLOSE', $4, now(), $5, now() + make_interval(secs => $6::double precision), $7) \
             RETURNING id",
        )
        .bind(ORG)
        .bind(branch.as_str())
        .bind(shift)
        .bind(counter)
        .bind(verifier)
        .bind(verified_offset_secs as f64)
        .bind(unique("cnt"))
        .fetch_one(pool)
        .await
        .expect("insert count");

        sqlx::query(
            "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, difference) \
             VALUES ($1, $2::uuid, $3, $4::numeric, $5::numeric, $6::numeric)",
        )
        .bind(id)
        .bind(ORG)
        .bind(currency)
        .bind(Decimal::from_str(counted).expect("counted dec"))
        .bind(Decimal::from_str(expected).expect("expected dec"))
        .bind(Decimal::from_str(difference).expect("diff dec"))
        .execute(pool)
        .await
        .expect("insert total");
    }

    #[tokio::test]
    async fn when_two_verified_close_counts_exist_the_latest_by_verification_time_wins() {
        let (pool, branch) = db().await;
        let sb = PgLedgerShiftBalances::new(pool.clone());
        let shift = create_shift(&pool, &branch).await;

        // Earlier verified CLOSE count (verified 10 seconds ago)
        seed_verified_close_count(
            &pool,
            &branch,
            &shift,
            -10,
            ("THB", "4000.00", "5000.00", "-1000.00"),
        )
        .await;

        // Later verified CLOSE count (verified at now)
        seed_verified_close_count(
            &pool,
            &branch,
            &shift,
            0,
            ("THB", "4995.00", "5000.00", "-5.00"),
        )
        .await;

        let balances = sb.shift_balances(&shift).await.expect("shift_balances");
        assert_eq!(balances.len(), 1);
        let thb_row = &balances[0];
        assert_eq!(thb_row.currency, CurrencyCode::THB);
        assert_eq!(
            thb_row.counted,
            Some(ForeignAmount::from_decimal(Decimal::new(499_500, 2)))
        );
        assert_eq!(
            thb_row.difference,
            Some(QuantityDelta::from_decimal(Decimal::new(-500, 2)).unwrap())
        );

        pool.close().await;
    }

    #[tokio::test]
    async fn three_decimal_currency_is_kept_and_excess_places_are_refused() {
        let (pool, branch) = db().await;
        let sb = PgLedgerShiftBalances::new(pool.clone());
        let shift = create_shift(&pool, &branch).await;

        seed_verified_close_count(
            &pool,
            &branch,
            &shift,
            0,
            ("KWD", "0.0250", "0.0000", "-0.0250"),
        )
        .await;

        let balances = sb.shift_balances(&shift).await.expect("shift_balances");
        assert_eq!(balances.len(), 1);
        let kwd_row = &balances[0];
        assert_eq!(kwd_row.currency, CurrencyCode::parse("KWD").unwrap());
        assert_eq!(
            kwd_row.counted,
            Some(ForeignAmount::from_decimal(Decimal::new(25, 3)))
        );
        assert_eq!(
            kwd_row.difference,
            Some(QuantityDelta::from_decimal(Decimal::new(-25, 3)).unwrap())
        );

        let shift_bad = create_shift(&pool, &branch).await;
        seed_verified_close_count(
            &pool,
            &branch,
            &shift_bad,
            0,
            ("THB", "0.0250", "0.0000", "-0.0250"),
        )
        .await;
        assert!(matches!(
            sb.shift_balances(&shift_bad).await,
            Err(AppError::Internal(ref msg)) if msg.contains("THB")
        ));

        pool.close().await;
    }
}
