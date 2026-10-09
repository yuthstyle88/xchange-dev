//! The `admin_reports` view (§7, §12.1).

use std::collections::BTreeSet;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::NaiveDate;
use pos_error::{AppError, AppResult};
use rust_decimal::prelude::FromStr;
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{fixed2, round_cents};

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
struct TxSqlRow {
    #[allow(dead_code)]
    document_ref: String,
    r#type: String,
    code: String,
    amount: String,
    rate: String,
    reference_rate: Option<String>,
    principal: String,
    fee: String,
    bank_fee: String,
    received: String,
    paid: String,
    method: String,
    staff_id: String,
    #[allow(dead_code)]
    value_date: String,
    #[allow(dead_code)]
    seq: i64,
}

#[derive(Debug)]
struct TxParsed {
    tx_type: String,
    code: String,
    amount: Decimal,
    #[allow(dead_code)]
    rate: Decimal,
    reference_rate: Option<Decimal>,
    principal: Decimal,
    fee: Decimal,
    bank_fee: Decimal,
    received: Decimal,
    paid: Decimal,
    method: String,
    staff: String,
}

fn parse_money(s: &str, field: &'static str) -> Result<Decimal, AppError> {
    Decimal::from_str(s).map_err(|e| {
        AppError::Internal(format!("corrupt money in transactions.{field} ({s}): {e}"))
    })
}

impl TryFrom<TxSqlRow> for TxParsed {
    type Error = AppError;

    fn try_from(r: TxSqlRow) -> Result<Self, Self::Error> {
        let method = if r.method == "SCB Transfer" {
            "Bank Transfer".to_string()
        } else {
            r.method
        };
        let amount = parse_money(&r.amount, "amount")?;
        let rate = parse_money(&r.rate, "rate")?;
        let reference_rate = match r.reference_rate {
            Some(ref s) if !s.trim().is_empty() => Some(parse_money(s, "reference_rate")?),
            _ => None,
        };
        let principal = parse_money(&r.principal, "principal")?;
        let fee = parse_money(&r.fee, "fee")?;
        let bank_fee = parse_money(&r.bank_fee, "bank_fee")?;
        let received = parse_money(&r.received, "received")?;
        let paid = parse_money(&r.paid, "paid")?;

        Ok(TxParsed {
            tx_type: r.r#type,
            code: r.code.trim().to_string(),
            amount,
            rate,
            reference_rate,
            principal,
            fee,
            bank_fee,
            received,
            paid,
            method,
            staff: r.staff_id,
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

            let sql_rows: Vec<TxSqlRow> = sqlx::query_as(
                "SELECT \
                    document_ref, \
                    type, \
                    code::text AS code, \
                    amount::text AS amount, \
                    rate::text AS rate, \
                    reference_rate::text AS reference_rate, \
                    principal::text AS principal, \
                    fee::text AS fee, \
                    bank_fee::text AS bank_fee, \
                    received::text AS received, \
                    paid::text AS paid, \
                    method, \
                    staff_id, \
                    value_date::text AS value_date, \
                    seq \
                 FROM transactions \
                 WHERE branch_id = $1 AND mode = 'LIVE' \
                   AND ($2::date IS NULL OR value_date >= $2::date) \
                   AND ($3::date IS NULL OR value_date <= $3::date) \
                 ORDER BY seq ASC",
            )
            .bind(&branch_id)
            .bind(from_date)
            .bind(to_date)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let parsed_rows: Vec<TxParsed> = sql_rows
                .into_iter()
                .map(TxParsed::try_from)
                .collect::<Result<Vec<_>, _>>()?;

            // 1. period
            let period = ReportPeriod {
                from: from_str,
                to: to_str,
            };

            // 2. stats
            let stats_received: Decimal = parsed_rows.iter().map(|r| r.received).sum();
            let stats_paid: Decimal = parsed_rows.iter().map(|r| r.paid).sum();
            let stats_fees: Decimal = parsed_rows.iter().map(|r| r.fee).sum();
            let stats_bank_fees: Decimal = parsed_rows.iter().map(|r| r.bank_fee).sum();
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
                    let m_rows: Vec<&TxParsed> =
                        parsed_rows.iter().filter(|r| r.method == m).collect();
                    let count = m_rows.len() as u64;
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
            for r in &parsed_rows {
                currency_set.insert(r.code.clone());
            }
            let by_currency = currency_set
                .into_iter()
                .map(|c| -> Result<ReportCurrency, AppError> {
                    let c_rows: Vec<&TxParsed> =
                        parsed_rows.iter().filter(|r| r.code == c).collect();
                    let buy_amount: Decimal = c_rows
                        .iter()
                        .filter(|r| r.tx_type == "BUY")
                        .map(|r| r.amount)
                        .sum();
                    let sell_amount: Decimal = c_rows
                        .iter()
                        .filter(|r| r.tx_type == "SELL")
                        .map(|r| r.amount)
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

            for r in &parsed_rows {
                if let Some(ref_rate) = r.reference_rate {
                    referenced += 1;
                    let val_cents = round_cents(r.amount * ref_rate)
                        .ok_or_else(|| AppError::Internal("reference value overflow".to_string()))?;
                    let val = Decimal::from_i128_with_scale(val_cents, 2);
                    let spread = if r.tx_type == "BUY" {
                        val - r.principal
                    } else {
                        r.principal - val
                    };
                    total_value += val;
                    total_spread += spread;
                } else {
                    unreferenced += 1;
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
            for r in &parsed_rows {
                staff_set.insert(r.staff.clone());
            }

            let mut by_employee = Vec::new();
            let mut employee_totals_candidates = Vec::new();

            for s in staff_set {
                let s_rows: Vec<&TxParsed> =
                    parsed_rows.iter().filter(|r| r.staff == s).collect();
                let count = s_rows.len() as u64;
                let completed = count;
                let received: Decimal = s_rows.iter().map(|r| r.received).sum();
                let paid: Decimal = s_rows.iter().map(|r| r.paid).sum();
                let net = received - paid;
                let fees: Decimal = s_rows.iter().map(|r| r.fee).sum();
                let bank_fees: Decimal = s_rows.iter().map(|r| r.bank_fee).sum();

                let methods = EMPLOYEE_CHANNELS
                    .iter()
                    .map(|&m| -> Result<ReportChannel, AppError> {
                        let m_rows: Vec<&TxParsed> =
                            s_rows.iter().copied().filter(|r| r.method == m).collect();
                        let m_count = m_rows.len() as u64;
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

                let mut seen_currencies = Vec::new();
                for r in &s_rows {
                    if !seen_currencies.contains(&r.code) {
                        seen_currencies.push(r.code.clone());
                    }
                }

                let currencies = seen_currencies
                    .into_iter()
                    .map(|c| -> Result<ReportEmployeeCurrency, AppError> {
                        let c_received: Decimal = s_rows
                            .iter()
                            .filter(|r| r.code == c && r.tx_type == "BUY")
                            .map(|r| r.amount)
                            .sum();
                        let c_paid: Decimal = s_rows
                            .iter()
                            .filter(|r| r.code == c && r.tx_type == "SELL")
                            .map(|r| r.amount)
                            .sum();
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

    fn sample_sql_row() -> TxSqlRow {
        TxSqlRow {
            document_ref: "XC-001".into(),
            r#type: "BUY".into(),
            code: "USD".into(),
            amount: "100.00".into(),
            rate: "30.0000".into(),
            reference_rate: Some("30.5000".into()),
            principal: "3000.00".into(),
            fee: "5.00".into(),
            bank_fee: "0.00".into(),
            received: "3005.00".into(),
            paid: "0.00".into(),
            method: "Cash".into(),
            staff_id: "dao".into(),
            value_date: "2026-10-06".into(),
            seq: 1,
        }
    }

    #[test]
    fn format_money_value_past_96_bits_returns_err_not_zero() {
        let res = format_money(Decimal::MAX);
        assert!(res.is_err(), "expected Err on value past 96 bits, got {:?}", res);
        match res.unwrap_err() {
            AppError::Internal(msg) => {
                assert!(msg.contains("overflow"));
            }
            other => panic!("expected AppError::Internal, got {:?}", other),
        }
    }

    #[test]
    fn bad_amount_fails_decode_rather_than_zero() {
        let mut row = sample_sql_row();
        row.amount = "x".into();
        let res = TxParsed::try_from(row);
        assert!(res.is_err(), "amount = 'x' must return Err, not default to zero");
        match res.unwrap_err() {
            AppError::Internal(msg) => {
                assert!(msg.contains("amount"));
                assert!(msg.contains("x"));
            }
            other => panic!("expected AppError::Internal, got {:?}", other),
        }
    }

    #[test]
    fn bad_rate_fails_decode() {
        let mut row = sample_sql_row();
        row.rate = "xyz".into();
        assert!(TxParsed::try_from(row).is_err());
    }

    #[test]
    fn bad_reference_rate_fails_decode() {
        let mut row = sample_sql_row();
        row.reference_rate = Some("bad_ref_rate".into());
        assert!(TxParsed::try_from(row).is_err());
    }

    #[test]
    fn bad_principal_fails_decode() {
        let mut row = sample_sql_row();
        row.principal = "invalid".into();
        assert!(TxParsed::try_from(row).is_err());
    }

    #[test]
    fn bad_fee_fails_decode() {
        let mut row = sample_sql_row();
        row.fee = "broken".into();
        assert!(TxParsed::try_from(row).is_err());
    }

    #[test]
    fn bad_bank_fee_fails_decode() {
        let mut row = sample_sql_row();
        row.bank_fee = "broken".into();
        assert!(TxParsed::try_from(row).is_err());
    }

    #[test]
    fn bad_received_fails_decode() {
        let mut row = sample_sql_row();
        row.received = "broken".into();
        assert!(TxParsed::try_from(row).is_err());
    }

    #[test]
    fn bad_paid_fails_decode() {
        let mut row = sample_sql_row();
        row.paid = "broken".into();
        assert!(TxParsed::try_from(row).is_err());
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
