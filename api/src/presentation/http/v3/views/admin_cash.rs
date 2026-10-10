//! The `admin_cash` view (§7, §12.1, §12.6, §12.9).

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::{Decimal, RoundingStrategy};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{
    fixed2, round_cents, CurrencyCode, ExchangeArithmetic, ForeignAmount, Rate, UnitSize,
};

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_cash::{
    AdminCashCountedItem, AdminCashData, AdminCashFilter, AdminCashReference, AdminCashShift,
    AdminCashStats, AdminCashStockRow,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

/// Computes the reference principal and spread for a transaction with a reference rate.
/// Rounds the reference value to cents using `xc_kernel::money::ExchangeArithmetic::thb`.
pub fn compute_reference_exchange(
    amount: Decimal,
    ref_rate: Decimal,
    unit_size: Decimal,
    principal: Decimal,
    is_buy: bool,
) -> AppResult<(Decimal, Decimal)> {
    // Keep amount.checked_mul(ref_rate) as an overflow guard before arithmetic.
    amount
        .checked_mul(ref_rate)
        .ok_or_else(|| AppError::Internal("amount overflow in reference rate calculation".into()))?;
    let foreign = ForeignAmount::from_decimal(amount);
    let ref_rate_parsed = Rate::parse(&ref_rate.normalize().to_string())
        .map_err(|e| AppError::Internal(format!("invalid reference rate: {e}")))?;
    let unit_val = UnitSize::from_decimal(unit_size.normalize())
        .map_err(|e| AppError::Internal(format!("invalid unit size: {e}")))?;
    let ref_principal = Decimal::from_i128_with_scale(
        ExchangeArithmetic::thb(foreign, ref_rate_parsed, unit_val).minor_i128(),
        2,
    );
    let spread = if is_buy {
        ref_principal - principal
    } else {
        principal - ref_principal
    };
    Ok((ref_principal, spread))
}

/// Computes the reference spread percentage with 2 decimal places using `MidpointAwayFromZero`.
pub fn compute_reference_percent(spread: Decimal, total_ref_value: Decimal) -> String {
    if total_ref_value.is_zero() {
        return "0.00".to_string();
    }
    let pct = (spread * Decimal::from(100) / total_ref_value)
        .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    format!("{:.2}", pct)
}

/// The 153 register currencies in register order: THB first, then A–Z.
pub fn register_currencies() -> Vec<String> {
    let mut rest: Vec<String> = CurrencyCode::all()
        .iter()
        .skip(1)
        .map(|c| c.as_str().to_string())
        .collect();
    rest.sort();
    let mut all = Vec::with_capacity(153);
    all.push("THB".to_string());
    all.extend(rest);
    all
}

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

struct AdminCashBranchView {
    pool: PgPool,
}

impl AdminCashBranchView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct OpenShiftRow {
    id: String,
    opened_at: DateTime<Utc>,
    opened_by: String,
}

#[derive(sqlx::FromRow)]
struct CountedRow {
    code: String,
    counted: Option<Decimal>,
}

#[derive(sqlx::FromRow)]
struct ShiftBalanceRow {
    code: String,
    opening: Decimal,
    expected: Decimal,
}

#[derive(sqlx::FromRow)]
struct TxRefRow {
    type_: String,
    principal: Decimal,
    amount: Decimal,
    reference_rate: Option<Decimal>,
    unit_size: Decimal,
}

impl BranchView for AdminCashBranchView {
    const NAME: &'static str = "admin_cash";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = true;
    type Data = AdminCashData;
    type Filter = AdminCashFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();
        let show_all = filter.all;

        async move {
            let currencies = register_currencies();

            // 1. Last counted from last closed shift
            let last_closed: Option<String> = sqlx::query_scalar(
                "SELECT id FROM shifts WHERE branch_id = $1 AND status = 'CLOSED' ORDER BY seq DESC LIMIT 1",
            )
            .bind(&branch_id)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?;

            let last_counted = if let Some(ref closed_id) = last_closed {
                let counted_rows: Vec<CountedRow> = sqlx::query_as(
                    "SELECT trim(code) as code, counted FROM shift_balances WHERE shift_id = $1",
                )
                .bind(closed_id)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?;

                let mut counted_map = BTreeMap::new();
                for r in counted_rows {
                    if let Some(c) = r.counted {
                        counted_map.insert(r.code, c);
                    }
                }

                let mut items = Vec::new();
                // THB always (even at zero)
                let thb_amount = counted_map.get("THB").copied().unwrap_or(Decimal::ZERO);
                items.push(AdminCashCountedItem {
                    code: "THB".to_string(),
                    amount: format!("{:.2}", thb_amount),
                });

                // Other currencies counted > 0, sorted A-Z
                let mut other_codes: Vec<String> = counted_map
                    .iter()
                    .filter(|(code, amt)| code.as_str() != "THB" && **amt > Decimal::ZERO)
                    .map(|(code, _)| code.clone())
                    .collect();
                other_codes.sort();

                for code in other_codes {
                    let amt = counted_map[&code];
                    items.push(AdminCashCountedItem {
                        code,
                        amount: format!("{:.2}", amt),
                    });
                }

                Some(items)
            } else {
                None
            };

            // 2. Open shift check
            let open_shift: Option<OpenShiftRow> = sqlx::query_as(
                "SELECT id, opened_at, opened_by FROM shifts WHERE branch_id = $1 AND status = 'OPEN' LIMIT 1",
            )
            .bind(&branch_id)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?;

            let Some(open_shift) = open_shift else {
                return Ok(AdminCashData {
                    shift: None,
                    stock: Vec::new(),
                    stats: None,
                    reference: AdminCashReference {
                        with_reference: 0,
                        without_reference: 0,
                        spread: "0.00".to_string(),
                        percent: "0.00".to_string(),
                    },
                    currencies,
                    last_counted,
                });
            };

            let shift_id = open_shift.id.clone();
            let shift_dto = AdminCashShift {
                id: open_shift.id,
                opened_at: open_shift.opened_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                staff: open_shift.opened_by,
            };

            // 3. Stock rows
            let balance_rows: Vec<ShiftBalanceRow> = sqlx::query_as(
                "SELECT trim(code) as code, opening, expected FROM shift_balances WHERE shift_id = $1",
            )
            .bind(&shift_id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let mut balance_map: BTreeMap<String, (Decimal, Decimal)> = BTreeMap::new();
            for r in balance_rows {
                balance_map.insert(r.code, (r.opening, r.expected));
            }

            let rated_codes: Vec<String> = sqlx::query_scalar(
                "SELECT DISTINCT trim(code) FROM rates WHERE branch_id = $1",
            )
            .bind(&branch_id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let codes_to_show = if show_all {
                register_currencies()
            } else {
                let mut candidates = BTreeSet::new();
                candidates.insert("THB".to_string());
                for c in rated_codes {
                    candidates.insert(c);
                }
                for (code, (opening, expected)) in &balance_map {
                    if !opening.is_zero() || !expected.is_zero() || (*expected - *opening) != Decimal::ZERO {
                        candidates.insert(code.clone());
                    }
                }
                sorted_currency_codes(candidates)
            };

            let mut stock = Vec::with_capacity(codes_to_show.len());
            for code in codes_to_show {
                let (opening, expected) = balance_map
                    .get(&code)
                    .copied()
                    .unwrap_or((Decimal::ZERO, Decimal::ZERO));
                let net = expected - opening;
                stock.push(AdminCashStockRow {
                    code,
                    opening: format!("{:.2}", opening),
                    net: format!("{:.2}", net),
                    expected: format!("{:.2}", expected),
                });
            }

            // 4. Stats (cash flow in THB)
            let thb_opening = balance_map
                .get("THB")
                .map(|(o, _)| *o)
                .unwrap_or(Decimal::ZERO);

            let cash_received: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(SUM(received), 0) FROM transactions WHERE shift_id = $1 AND method = 'Cash'",
            )
            .bind(&shift_id)
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;

            let thb_in: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount), 0) FROM stock_movements WHERE shift_id = $1 AND code = 'THB' AND direction = 'IN'",
            )
            .bind(&shift_id)
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;

            let incoming = cash_received + thb_in;

            let cash_paid: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(SUM(paid), 0) FROM transactions WHERE shift_id = $1 AND method = 'Cash'",
            )
            .bind(&shift_id)
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;

            let thb_out: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount), 0) FROM stock_movements WHERE shift_id = $1 AND code = 'THB' AND direction = 'OUT'",
            )
            .bind(&shift_id)
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;

            let outgoing = cash_paid + thb_out;
            let remaining = thb_opening + incoming - outgoing;

            let stats = Some(AdminCashStats {
                opening: format!("{:.2}", thb_opening),
                incoming: format!("{:.2}", incoming),
                outgoing: format!("{:.2}", outgoing),
                remaining: format!("{:.2}", remaining),
            });

            // 5. Reference summary over open shift's exchanges
            let tx_rows: Vec<TxRefRow> = sqlx::query_as(
                "SELECT type as type_, principal, amount, reference_rate, unit_size FROM transactions WHERE shift_id = $1",
            )
            .bind(&shift_id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let mut with_reference: u32 = 0;
            let mut without_reference: u32 = 0;
            let mut total_spread = Decimal::ZERO;
            let mut total_ref_value = Decimal::ZERO;

            for tx in tx_rows {
                if let Some(ref_rate) = tx.reference_rate {
                    with_reference += 1;
                    let (ref_principal, spread) = compute_reference_exchange(
                        tx.amount,
                        ref_rate,
                        tx.unit_size,
                        tx.principal,
                        tx.type_ == "BUY",
                    )?;
                    total_spread += spread;
                    total_ref_value += ref_principal;
                } else {
                    without_reference += 1;
                }
            }

            let percent = compute_reference_percent(total_spread, total_ref_value);
            let spread_cents = round_cents(total_spread)
                .ok_or_else(|| AppError::Internal("spread overflow in reference rate calculation".into()))?;
            let reference = AdminCashReference {
                with_reference,
                without_reference,
                spread: fixed2(spread_cents),
                percent,
            };

            Ok(AdminCashData {
                shift: Some(shift_dto),
                stock,
                stats,
                reference,
                currencies,
                last_counted,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(AdminCashBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminCashFilter::export_all(&cfg)?;
    AdminCashShift::export_all(&cfg)?;
    AdminCashStockRow::export_all(&cfg)?;
    AdminCashStats::export_all(&cfg)?;
    AdminCashReference::export_all(&cfg)?;
    AdminCashCountedItem::export_all(&cfg)?;
    AdminCashData::export_all(&cfg)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_midpoint_away_from_zero_rounding() {
        // 2.525 rounds to 2.53 under MidpointAwayFromZero (differs from half-even 2.52)
        let spread = Decimal::from_str("2.525").unwrap();
        let total_ref_value = Decimal::from(100);
        let result = compute_reference_percent(spread, total_ref_value);
        assert_eq!(result, "2.53");
    }

    #[test]
    fn test_percent_two_decimal_places_formatting() {
        // 1.5% must format as "1.50", not "1.5" (goes red if {:.2} becomes to_string())
        let spread = Decimal::from_str("1.5").unwrap();
        let total_ref_value = Decimal::from(100);
        let result = compute_reference_percent(spread, total_ref_value);
        assert_eq!(result, "1.50");
    }

    #[test]
    fn test_reference_exchange_midpoint_rounding() {
        // 10.5 * 0.05 = 0.525: round_cents rounds to 0.53, whereas banker's round_dp(2) rounds to 0.52
        let amount = Decimal::from_str("10.5").unwrap();
        let ref_rate = Decimal::from_str("0.05").unwrap();
        let principal = Decimal::from_str("0.50").unwrap();
        let (ref_principal, spread) =
            compute_reference_exchange(amount, ref_rate, Decimal::ONE, principal, true).unwrap();
        assert_eq!(ref_principal, Decimal::from_str("0.53").unwrap());
        assert_eq!(spread, Decimal::from_str("0.03").unwrap());
    }

    #[test]
    fn test_reference_exchange_divides_by_unit_size() {
        // 1000 JPY at ref_rate 23.5 per 100 gives ref_principal 235.00
        let amount = Decimal::from_str("1000").unwrap();
        let ref_rate = Decimal::from_str("23.5").unwrap();
        let unit_size = Decimal::from_str("100").unwrap();
        let principal = Decimal::from_str("230.00").unwrap();
        let (ref_principal, spread) =
            compute_reference_exchange(amount, ref_rate, unit_size, principal, true).unwrap();
        assert_eq!(ref_principal, Decimal::from_str("235.00").unwrap());
        assert_eq!(spread, Decimal::from_str("5.00").unwrap());
    }

    #[test]
    fn test_reference_exchange_overflow_returns_err() {
        // numeric(20,2) x numeric(14,4) can exceed 96 bits.
        // 1e18 x 1e10: cents overflow 96 bits, must return Err, not Decimal::ZERO / 0.
        let amount = Decimal::from_str("1000000000000000000").unwrap(); // 1e18
        let ref_rate = Decimal::from_str("10000000000").unwrap(); // 1e10
        let principal = Decimal::ZERO;
        let result = compute_reference_exchange(amount, ref_rate, Decimal::ONE, principal, true);
        assert!(result.is_err(), "expected overflow Err, got {:?}", result);
    }
}
