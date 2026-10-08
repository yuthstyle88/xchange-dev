//! The `admin_shifts` view (§7, §12.6).

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_shifts::{
    AdminShiftBalanceRow, AdminShiftDifferenceItem, AdminShiftRow,
};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

/// Sorts currency codes so that THB is first (if present), followed by A–Z.
fn sorted_currency_codes<I: IntoIterator<Item = S>, S: AsRef<str>>(codes: I) -> Vec<String> {
    let mut has_thb = false;
    let mut rest = Vec::new();
    for c in codes {
        let s = c.as_ref();
        if s == "THB" {
            has_thb = true;
        } else {
            rest.push(s.to_string());
        }
    }
    rest.sort();
    rest.dedup();
    let mut result = Vec::with_capacity(rest.len() + usize::from(has_thb));
    if has_thb {
        result.push("THB".to_string());
    }
    result.extend(rest);
    result
}

struct AdminShiftsPagedView {
    pool: PgPool,
}

impl AdminShiftsPagedView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct ShiftSqlRow {
    id: String,
    status: String,
    opened_at: DateTime<Utc>,
    opened_by: String,
    close_note: String,
}

#[derive(sqlx::FromRow)]
struct BalanceSqlRow {
    code: String,
    opening: Decimal,
    expected: Decimal,
    counted: Option<Decimal>,
    difference: Option<Decimal>,
}

impl PagedView for AdminShiftsPagedView {
    const NAME: &'static str = "admin_shifts";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = AdminShiftRow;
    type Filter = NoFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();

        async move {
            let shifts: Vec<ShiftSqlRow> = if let Some(before_id) = before {
                let before_seq: Option<i64> = sqlx::query_scalar(
                    "SELECT seq FROM shifts WHERE id = $1 AND branch_id = $2",
                )
                .bind(&before_id)
                .bind(&branch_id)
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?;

                let Some(before_seq) = before_seq else {
                    return Ok(vec![]);
                };

                sqlx::query_as(
                    "SELECT id, status, opened_at, opened_by, close_note \
                     FROM shifts \
                     WHERE branch_id = $1 AND seq < $2 \
                     ORDER BY seq DESC \
                     LIMIT $3",
                )
                .bind(&branch_id)
                .bind(before_seq)
                .bind(limit as i64)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            } else {
                sqlx::query_as(
                    "SELECT id, status, opened_at, opened_by, close_note \
                     FROM shifts \
                     WHERE branch_id = $1 \
                     ORDER BY seq DESC \
                     LIMIT $2",
                )
                .bind(&branch_id)
                .bind(limit as i64)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            };

            let mut result = Vec::with_capacity(shifts.len());

            for s in shifts {
                let is_closed = s.status == "CLOSED";

                let balance_rows: Vec<BalanceSqlRow> = sqlx::query_as(
                    "SELECT trim(code) as code, opening, expected, counted, difference \
                     FROM shift_balances \
                     WHERE shift_id = $1",
                )
                .bind(&s.id)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?;

                let mut b_map: BTreeMap<String, BalanceSqlRow> = BTreeMap::new();
                for b in balance_rows {
                    b_map.insert(b.code.clone(), b);
                }

                if is_closed {
                    // Difference items: non-zero difference, THB first then A-Z
                    let mut diff_codes = BTreeSet::new();
                    for (code, b) in &b_map {
                        if b.difference.is_some_and(|diff| !diff.is_zero()) {
                            diff_codes.insert(code.clone());
                        }
                    }
                    let sorted_diff_codes = sorted_currency_codes(diff_codes);
                    let diff_items: Vec<AdminShiftDifferenceItem> = sorted_diff_codes
                        .into_iter()
                        .map(|code| {
                            let amt = b_map[&code].difference.unwrap_or(Decimal::ZERO);
                            AdminShiftDifferenceItem {
                                code,
                                amount: format!("{:.2}", amt),
                            }
                        })
                        .collect();

                    // Balance rows: THB always, plus any currency with non-zero opening, expected, counted, or difference
                    let mut bal_codes = BTreeSet::new();
                    bal_codes.insert("THB".to_string());
                    for (code, b) in &b_map {
                        let c_non_zero = b.counted.is_some_and(|c| !c.is_zero());
                        let d_non_zero = b.difference.is_some_and(|d| !d.is_zero());
                        if !b.opening.is_zero() || !b.expected.is_zero() || c_non_zero || d_non_zero {
                            bal_codes.insert(code.clone());
                        }
                    }
                    let sorted_bal_codes = sorted_currency_codes(bal_codes);
                    let balances: Vec<AdminShiftBalanceRow> = sorted_bal_codes
                        .into_iter()
                        .map(|code| {
                            let (exp, cnt, diff) = match b_map.get(&code) {
                                Some(b) => (
                                    format!("{:.2}", b.expected),
                                    b.counted.map(|c| format!("{:.2}", c)),
                                    b.difference.map(|d| format!("{:.2}", d)),
                                ),
                                None => ("0.00".to_string(), None, None),
                            };
                            AdminShiftBalanceRow {
                                code,
                                expected: exp,
                                counted: cnt,
                                difference: diff,
                            }
                        })
                        .collect();

                    result.push(AdminShiftRow {
                        id: s.id,
                        status: s.status,
                        opened_at: s.opened_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                        staff: s.opened_by,
                        note: s.close_note,
                        difference: Some(diff_items),
                        balances,
                    });
                } else {
                    // Shift is OPEN: difference is None, note is empty string
                    let mut bal_codes = BTreeSet::new();
                    bal_codes.insert("THB".to_string());
                    for (code, b) in &b_map {
                        if !b.opening.is_zero() || !b.expected.is_zero() {
                            bal_codes.insert(code.clone());
                        }
                    }
                    let sorted_bal_codes = sorted_currency_codes(bal_codes);
                    let balances: Vec<AdminShiftBalanceRow> = sorted_bal_codes
                        .into_iter()
                        .map(|code| {
                            let exp = match b_map.get(&code) {
                                Some(b) => format!("{:.2}", b.expected),
                                None => "0.00".to_string(),
                            };
                            AdminShiftBalanceRow {
                                code,
                                expected: exp,
                                counted: None,
                                difference: None,
                            }
                        })
                        .collect();

                    result.push(AdminShiftRow {
                        id: s.id,
                        status: s.status,
                        opened_at: s.opened_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                        staff: s.opened_by,
                        note: String::new(),
                        difference: None,
                        balances,
                    });
                }
            }

            Ok(result)
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(cfg, Arc::new(AdminShiftsPagedView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminShiftDifferenceItem::export_all(&cfg)?;
    AdminShiftBalanceRow::export_all(&cfg)?;
    AdminShiftRow::export_all(&cfg)?;
    Ok(())
}
