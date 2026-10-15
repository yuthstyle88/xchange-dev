//! The `admin_reports` view (§7, §12.1).

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::NaiveDate;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{ExchangeArithmetic, ForeignAmount, Rate, UnitSize, fixed2, round_cents};

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_reports::*;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

const CHANNELS: [&str; 5] = [
    "Cash",
    "Bank Transfer",
    "Credit Card",
    "Alipay",
    "PromptPay",
];

const EMPLOYEE_CHANNELS: [&str; 5] = [
    "Cash",
    "Credit Card",
    "Alipay",
    "PromptPay",
    "Bank Transfer",
];

fn format_money(d: Decimal) -> Result<String, AppError> {
    let cents = round_cents(d)
        .ok_or_else(|| AppError::Internal("money formatting overflow".to_string()))?;
    Ok(fixed2(cents))
}

struct AdminReportsBranchView {
    pool: PgPool,
}

impl AdminReportsBranchView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct StaffMethodRow {
    staff_id: String,
    method: String,
    exchanges: i64,
    received: Decimal,
    paid: Decimal,
    fee: Decimal,
    bank_fee: Decimal,
}

#[derive(sqlx::FromRow)]
struct StaffCurrencySideRow {
    staff_id: String,
    code: String,
    shop_side: String,
    quantity: Decimal,
    principal: Decimal,
    fee: Decimal,
    first_rn: i64,
}

#[derive(sqlx::FromRow)]
struct RefSqlRow {
    shop_side: String,
    quantity: Decimal,
    reference_rate: Option<String>,
    unit_size: String,
    line_count: i64,
    principal: Decimal,
}

#[derive(Debug)]
struct RefGroup {
    shop_side: String,
    quantity: Decimal,
    reference_rate: Option<Rate>,
    unit_size: UnitSize,
    line_count: u64,
    principal: Decimal,
}

impl TryFrom<RefSqlRow> for RefGroup {
    type Error = AppError;

    fn try_from(r: RefSqlRow) -> Result<Self, Self::Error> {
        let reference_rate = match r.reference_rate {
            Some(ref s) if !s.trim().is_empty() => Some(Rate::parse(s.trim()).map_err(|e| {
                AppError::Internal(format!(
                    "corrupt reference_rate in transactions.reference_rate ({s}): {e}"
                ))
            })?),
            _ => None,
        };
        let unit_size = UnitSize::parse(r.unit_size.trim()).map_err(|e| {
            AppError::Internal(format!(
                "corrupt unit_size in transactions.unit_size ({}): {e}",
                r.unit_size
            ))
        })?;
        let line_count = u64::try_from(r.line_count).map_err(|e| {
            AppError::Internal(format!(
                "corrupt line_count in reference group ({}): {e}",
                r.line_count
            ))
        })?;
        Ok(RefGroup {
            shop_side: r.shop_side,
            quantity: r.quantity,
            reference_rate,
            unit_size,
            line_count,
            principal: r.principal,
        })
    }
}

impl BranchView for AdminReportsBranchView {
    const NAME: &'static str = "admin_reports";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = AdminReportsData;
    type Filter = AdminReportsFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();
        let from_str = filter.from.clone();
        let to_str = filter.to.clone();
        let filter_staff = filter.staff.clone();
        let filter_staff_q = filter.staff_q.clone();

        async move {
            let from_date: Option<NaiveDate> = if !from_str.is_empty() {
                NaiveDate::parse_from_str(&from_str, "%Y-%m-%d").ok()
            } else {
                None
            };
            let to_date: Option<NaiveDate> = if !to_str.is_empty() {
                NaiveDate::parse_from_str(&to_str, "%Y-%m-%d").ok()
            } else {
                None
            };

            let mut tx = pool.begin().await.map_err(AppError::Database)?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;

            // Q1: per (staff_id, folded method), one row per group, counting each exchange and
            // its payment 1 bank fee once.
            let q1_rows: Vec<StaffMethodRow> = sqlx::query_as(
                "SELECT \
                    t.staff_id, \
                    CASE p.method WHEN 'SCB Transfer' THEN 'Bank Transfer' ELSE p.method END AS method, \
                    count(*) AS exchanges, \
                    COALESCE(sum(lx.received), 0) AS received, \
                    COALESCE(sum(lx.paid), 0) AS paid, \
                    COALESCE(sum(lx.fee), 0) AS fee, \
                    COALESCE(sum(p.bank_fee_thb), 0) AS bank_fee \
                 FROM transactions t \
                 JOIN transaction_payments p ON p.transaction_id = t.id AND p.payment_no = 1 \
                 JOIN LATERAL ( \
                    SELECT \
                        COALESCE(sum(l.settle_thb) FILTER (WHERE l.shop_side = 'SELL'), 0) AS received, \
                        COALESCE(sum(l.settle_thb) FILTER (WHERE l.shop_side = 'BUY'), 0) AS paid, \
                        COALESCE(sum(l.fee_thb), 0) AS fee \
                    FROM transaction_lines l \
                    WHERE l.transaction_id = t.id \
                 ) lx ON true \
                 WHERE t.branch_id = $1 AND t.mode = 'LIVE' AND t.status = 'COMPLETED' \
                   AND ($2::date IS NULL OR t.value_date >= $2::date) \
                   AND ($3::date IS NULL OR t.value_date <= $3::date) \
                 GROUP BY 1, 2",
            )
            .bind(&branch_id)
            .bind(from_date)
            .bind(to_date)
            .fetch_all(&mut *tx)
            .await
            .map_err(AppError::Database)?;

            // Q2: per (staff_id, currency, shop_side), with first-appearance order per staff.
            let q2_rows: Vec<StaffCurrencySideRow> = sqlx::query_as(
                "SELECT \
                    staff_id, \
                    code, \
                    shop_side, \
                    COALESCE(sum(quantity), 0) AS quantity, \
                    COALESCE(sum(thb_amount), 0) AS principal, \
                    COALESCE(sum(fee_thb), 0) AS fee, \
                    min(rn) AS first_rn \
                 FROM ( \
                    SELECT \
                        t.staff_id, \
                        trim(l.currency::text) AS code, \
                        l.shop_side, \
                        l.quantity, \
                        l.thb_amount, \
                        l.fee_thb, \
                        row_number() OVER (PARTITION BY t.staff_id ORDER BY t.seq ASC, l.line_no ASC) AS rn \
                    FROM transactions t \
                    JOIN transaction_lines l ON l.transaction_id = t.id \
                    JOIN transaction_payments p ON p.transaction_id = t.id AND p.payment_no = 1 \
                    WHERE t.branch_id = $1 AND t.mode = 'LIVE' AND t.status = 'COMPLETED' \
                      AND ($2::date IS NULL OR t.value_date >= $2::date) \
                      AND ($3::date IS NULL OR t.value_date <= $3::date) \
                 ) ranked \
                 GROUP BY staff_id, code, shop_side",
            )
            .bind(&branch_id)
            .bind(from_date)
            .bind(to_date)
            .fetch_all(&mut *tx)
            .await
            .map_err(AppError::Database)?;

            // Q3: reference section grouped by (shop_side, quantity, reference_rate, unit_size).
            let q3_sql_rows: Vec<RefSqlRow> = sqlx::query_as(
                "SELECT \
                    l.shop_side, \
                    l.quantity, \
                    l.reference_rate::text AS reference_rate, \
                    l.unit_size::text AS unit_size, \
                    count(*) AS line_count, \
                    COALESCE(sum(l.thb_amount), 0) AS principal \
                 FROM transactions t \
                 JOIN transaction_lines l ON l.transaction_id = t.id \
                 JOIN transaction_payments p ON p.transaction_id = t.id AND p.payment_no = 1 \
                 WHERE t.branch_id = $1 AND t.mode = 'LIVE' AND t.status = 'COMPLETED' \
                   AND ($2::date IS NULL OR t.value_date >= $2::date) \
                   AND ($3::date IS NULL OR t.value_date <= $3::date) \
                 GROUP BY l.shop_side, l.quantity, l.reference_rate, l.unit_size",
            )
            .bind(&branch_id)
            .bind(from_date)
            .bind(to_date)
            .fetch_all(&mut *tx)
            .await
            .map_err(AppError::Database)?;

            tx.commit().await.map_err(AppError::Database)?;

            let q3_rows: Vec<RefGroup> = q3_sql_rows
                .into_iter()
                .map(RefGroup::try_from)
                .collect::<Result<Vec<_>, _>>()?;

            // 1. period
            let period = ReportPeriod {
                from: from_str,
                to: to_str,
            };

            // 2. stats
            let stats_received: Decimal = q1_rows.iter().map(|r| r.received).sum();
            let stats_paid: Decimal = q1_rows.iter().map(|r| r.paid).sum();
            let stats_fees: Decimal = q1_rows.iter().map(|r| r.fee).sum();
            let stats_bank_fees: Decimal = q1_rows.iter().map(|r| r.bank_fee).sum();
            let stats = ReportStats {
                received: format_money(stats_received)?,
                paid: format_money(stats_paid)?,
                fees: format_money(stats_fees)?,
                bank_fees: format_money(stats_bank_fees)?,
            };

            // 3. byChannel
            let by_channel = CHANNELS
                .iter()
                .map(|&m| -> Result<ReportChannel, AppError> {
                    let m_rows: Vec<&StaffMethodRow> =
                        q1_rows.iter().filter(|r| r.method == m).collect();
                    let count: u64 = m_rows.iter().map(|r| r.exchanges as u64).sum();
                    let received: Decimal = m_rows.iter().map(|r| r.received).sum();
                    let paid: Decimal = m_rows.iter().map(|r| r.paid).sum();
                    let net = received - paid;
                    Ok(ReportChannel {
                        method: m.to_string(),
                        count,
                        received: format_money(received)?,
                        paid: format_money(paid)?,
                        net: format_money(net)?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            // 4. byCurrency
            let mut currency_set = BTreeSet::new();
            for r in &q2_rows {
                currency_set.insert(r.code.clone());
            }
            let by_currency = currency_set
                .into_iter()
                .map(|c| -> Result<ReportCurrency, AppError> {
                    let c_rows: Vec<&StaffCurrencySideRow> =
                        q2_rows.iter().filter(|r| r.code == c).collect();
                    let buy_amount: Decimal = c_rows
                        .iter()
                        .filter(|r| r.shop_side == "BUY")
                        .map(|r| r.quantity)
                        .sum();
                    let sell_amount: Decimal = c_rows
                        .iter()
                        .filter(|r| r.shop_side == "SELL")
                        .map(|r| r.quantity)
                        .sum();
                    let principal: Decimal = c_rows.iter().map(|r| r.principal).sum();
                    let fees: Decimal = c_rows.iter().map(|r| r.fee).sum();
                    Ok(ReportCurrency {
                        currency: c,
                        buy_amount: format_money(buy_amount)?,
                        sell_amount: format_money(sell_amount)?,
                        principal: format_money(principal)?,
                        fees: format_money(fees)?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            // 5. reference
            let mut referenced = 0u64;
            let mut unreferenced = 0u64;
            let mut total_value = Decimal::ZERO;
            let mut total_spread = Decimal::ZERO;

            for r in &q3_rows {
                if let Some(ref_rate) = r.reference_rate {
                    referenced += r.line_count;
                    let foreign = ForeignAmount::from_decimal(r.quantity);
                    let per_line_val = Decimal::from_i128_with_scale(
                        ExchangeArithmetic::thb(foreign, ref_rate, r.unit_size).minor_i128(),
                        2,
                    );
                    let val = per_line_val * Decimal::from(r.line_count);
                    let spread = if r.shop_side == "BUY" {
                        val - r.principal
                    } else {
                        r.principal - val
                    };
                    total_value += val;
                    total_spread += spread;
                } else {
                    unreferenced += r.line_count;
                }
            }

            let (value_str, spread_str, percent_opt) = if referenced == 0 {
                (
                    format_money(total_value)?,
                    format_money(total_spread)?,
                    None,
                )
            } else if total_value.is_zero() {
                (
                    format_money(total_value)?,
                    format_money(total_spread)?,
                    Some("0.00".to_string()),
                )
            } else {
                let pct_exact = total_spread / total_value * Decimal::from(100);
                let pct_cents = round_cents(pct_exact)
                    .ok_or_else(|| AppError::Internal("percent overflow".to_string()))?;
                (
                    format_money(total_value)?,
                    format_money(total_spread)?,
                    Some(fixed2(pct_cents)),
                )
            };
            let reference = ReportReference {
                referenced,
                unreferenced,
                value: value_str,
                spread: spread_str,
                percent: percent_opt,
            };

            // 6. byEmployee
            let mut staff_set = BTreeSet::new();
            for r in &q1_rows {
                staff_set.insert(r.staff_id.clone());
            }

            let mut by_employee = Vec::new();
            let mut employee_totals_candidates = Vec::new();

            for s in staff_set {
                let s_rows: Vec<&StaffMethodRow> =
                    q1_rows.iter().filter(|r| r.staff_id == s).collect();
                let count: u64 = s_rows.iter().map(|r| r.exchanges as u64).sum();
                let completed = count;
                let received: Decimal = s_rows.iter().map(|r| r.received).sum();
                let paid: Decimal = s_rows.iter().map(|r| r.paid).sum();
                let net = received - paid;
                let fees: Decimal = s_rows.iter().map(|r| r.fee).sum();
                let bank_fees: Decimal = s_rows.iter().map(|r| r.bank_fee).sum();

                let methods = EMPLOYEE_CHANNELS
                    .iter()
                    .map(|&m| -> Result<ReportChannel, AppError> {
                        let m_rows: Vec<&StaffMethodRow> =
                            s_rows.iter().copied().filter(|r| r.method == m).collect();
                        let m_count: u64 = m_rows.iter().map(|r| r.exchanges as u64).sum();
                        let m_received: Decimal = m_rows.iter().map(|r| r.received).sum();
                        let m_paid: Decimal = m_rows.iter().map(|r| r.paid).sum();
                        let m_net = m_received - m_paid;
                        Ok(ReportChannel {
                            method: m.to_string(),
                            count: m_count,
                            received: format_money(m_received)?,
                            paid: format_money(m_paid)?,
                            net: format_money(m_net)?,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let mut emp_currencies: BTreeMap<String, (i64, Decimal, Decimal)> = BTreeMap::new();
                for r in q2_rows.iter().filter(|r| r.staff_id == s) {
                    let entry = emp_currencies.entry(r.code.clone()).or_insert((
                        r.first_rn,
                        Decimal::ZERO,
                        Decimal::ZERO,
                    ));
                    if r.first_rn < entry.0 {
                        entry.0 = r.first_rn;
                    }
                    if r.shop_side == "BUY" {
                        entry.1 += r.quantity;
                    } else if r.shop_side == "SELL" {
                        entry.2 += r.quantity;
                    }
                }
                let mut ordered_currencies: Vec<(String, i64, Decimal, Decimal)> = emp_currencies
                    .into_iter()
                    .map(|(code, (rn, rec, paid_amt))| (code, rn, rec, paid_amt))
                    .collect();
                ordered_currencies.sort_by_key(|(_, rn, _, _)| *rn);

                let currencies = ordered_currencies
                    .into_iter()
                    .map(|(c, _, c_received, c_paid)| -> Result<ReportEmployeeCurrency, AppError> {
                        Ok(ReportEmployeeCurrency {
                            currency: c,
                            received: format_money(c_received)?,
                            paid: format_money(c_paid)?,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                by_employee.push(ReportEmployeeGroup {
                    staff: s.clone(),
                    count,
                    completed,
                    pending: 0,
                    cancelled: 0,
                    received: format_money(received)?,
                    paid: format_money(paid)?,
                    net: format_money(net)?,
                    fees: format_money(fees)?,
                    bank_fees: format_money(bank_fees)?,
                    methods,
                    currencies,
                });

                employee_totals_candidates.push((s, completed, received, paid));
            }

            // 7. employeeTotals
            let mut people = 0u64;
            let mut completed = 0u64;
            let mut tot_received = Decimal::ZERO;
            let mut tot_paid = Decimal::ZERO;

            for (staff, comp, rec, paid) in employee_totals_candidates {
                if !filter_staff.is_empty() && staff != filter_staff {
                    continue;
                }
                if !filter_staff_q.is_empty()
                    && !staff
                        .to_lowercase()
                        .contains(&filter_staff_q.to_lowercase())
                {
                    continue;
                }
                people += 1;
                completed += comp;
                tot_received += rec;
                tot_paid += paid;
            }

            let employee_totals = if people == 0 {
                ReportEmployeeTotals {
                    people: 0,
                    completed: 0,
                    received: "0.00".to_string(),
                    paid: "0.00".to_string(),
                    net: "0.00".to_string(),
                }
            } else {
                let tot_net = tot_received - tot_paid;
                ReportEmployeeTotals {
                    people,
                    completed,
                    received: format_money(tot_received)?,
                    paid: format_money(tot_paid)?,
                    net: format_money(tot_net)?,
                }
            };

            Ok(AdminReportsData {
                period,
                stats,
                by_channel,
                by_currency,
                reference,
                by_employee,
                employee_totals,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(AdminReportsBranchView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ReportPeriod::export_all(&cfg)?;
    ReportStats::export_all(&cfg)?;
    ReportChannel::export_all(&cfg)?;
    ReportCurrency::export_all(&cfg)?;
    ReportReference::export_all(&cfg)?;
    ReportEmployeeCurrency::export_all(&cfg)?;
    ReportEmployeeGroup::export_all(&cfg)?;
    ReportEmployeeTotals::export_all(&cfg)?;
    AdminReportsData::export_all(&cfg)?;
    AdminReportsFilter::export_all(&cfg)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::StatusCode;
    use crate::presentation::http::v3::view::ViewFilter;

    #[test]
    fn staff_q_longer_than_100_characters_is_refused_naming_staff_q() {
        let long_q = "a".repeat(101);
        let filter = AdminReportsFilter {
            from: String::new(),
            to: String::new(),
            staff: String::new(),
            staff_q: long_q,
        };
        let err = filter.check().unwrap_err();
        assert_eq!(err.code, "invalidRequest");
        assert_eq!(err.field, Some("staffQ"));
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        let ok_filter = AdminReportsFilter {
            from: String::new(),
            to: String::new(),
            staff: String::new(),
            staff_q: "a".repeat(100),
        };
        assert!(ok_filter.check().is_ok());
    }

    fn sample_sql_row() -> RefSqlRow {
        RefSqlRow {
            shop_side: "BUY".into(),
            quantity: Decimal::from(100),
            reference_rate: Some("30.5000".into()),
            unit_size: "1".into(),
            line_count: 1,
            principal: Decimal::from(3000),
        }
    }

    #[test]
    fn bad_unit_size_fails_decode() {
        let mut row = sample_sql_row();
        row.unit_size = "broken".into();
        let res = RefGroup::try_from(row);
        assert!(res.is_err(), "broken unit_size must return Err");
        match res.unwrap_err() {
            AppError::Internal(msg) => {
                assert!(msg.contains("unit_size"), "{msg}");
                assert!(msg.contains("broken"), "{msg}");
            }
            other => panic!("expected AppError::Internal, got {:?}", other),
        }
    }

    #[test]
    fn zero_unit_size_fails_decode() {
        let mut row = sample_sql_row();
        row.unit_size = "0".into();
        let res = RefGroup::try_from(row);
        assert!(res.is_err());
        match res.unwrap_err() {
            AppError::Internal(msg) => assert!(msg.contains("unit_size"), "{msg}"),
            other => panic!("expected AppError::Internal, got {:?}", other),
        }
    }

    #[test]
    fn format_money_value_past_96_bits_returns_err_not_zero() {
        let res = format_money(Decimal::MAX);
        assert!(
            res.is_err(),
            "expected Err on value past 96 bits, got {:?}",
            res
        );
        match res.unwrap_err() {
            AppError::Internal(msg) => {
                assert!(msg.contains("overflow"));
            }
            other => panic!("expected AppError::Internal, got {:?}", other),
        }
    }

    #[test]
    fn bad_reference_rate_fails_decode() {
        let mut row = sample_sql_row();
        row.reference_rate = Some("bad_ref_rate".into());
        assert!(RefGroup::try_from(row).is_err());
    }

    #[test]
    fn non_canonical_date_such_as_single_digit_month_is_refused() {
        let filter = AdminReportsFilter {
            from: "2026-1-5".into(),
            to: "2026-10-06".into(),
            staff: "".into(),
            staff_q: "".into(),
        };
        let err = filter.check().unwrap_err();
        assert_eq!(err.code, "invalidRequest");
        assert_eq!(err.field, Some("from"));
    }
}
