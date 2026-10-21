//! The `admin_cash` view (§7, §12.1, §12.6, §12.9).

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{
    CurrencyCode, ExchangeArithmetic, ForeignAmount, Money, MoneyDelta, Rate, UnitSize, fixed2,
    round_cents,
};
use xc_shift::domain::shift::repositories::shift_balances::{ShiftBalance, ShiftBalances};
use xc_shift::domain::shift::repositories::shift_repository::ShiftRepository;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_cash::{
    AdminCashCountedItem, AdminCashData, AdminCashFilter, AdminCashReference, AdminCashStats,
    AdminCashStockRow,
};
use crate::presentation::dto::v3::common::OpenShiftDto;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

fn checked_add_money(a: Money, b: Money) -> AppResult<Money> {
    let sum = a
        .minor_i128()
        .checked_add(b.minor_i128())
        .ok_or_else(|| AppError::Internal("money sum overflow in admin_cash".into()))?;
    Money::try_from_minor_i128(sum)
        .map_err(|_| AppError::Internal("money sum overflow in admin_cash".into()))
}

/// Computes the reference principal and spread for a transaction with a reference rate.
/// Rounds the reference value to cents using `xc_kernel::money::ExchangeArithmetic::try_thb`.
pub fn compute_reference_exchange(
    amount: Decimal,
    ref_rate: Decimal,
    unit_size: Decimal,
    principal: Decimal,
    is_buy: bool,
) -> AppResult<(Decimal, Decimal)> {
    // Keep amount.checked_mul(ref_rate) as an overflow guard before arithmetic.
    amount.checked_mul(ref_rate).ok_or_else(|| {
        AppError::Internal("amount overflow in reference rate calculation".into())
    })?;
    let foreign = ForeignAmount::from_decimal(amount);
    let ref_rate_parsed = Rate::parse(&ref_rate.normalize().to_string())
        .map_err(|e| AppError::Internal(format!("invalid reference rate: {e}")))?;
    let unit_val = UnitSize::from_decimal(unit_size.normalize())
        .map_err(|e| AppError::Internal(format!("invalid unit size: {e}")))?;
    let ref_thb = ExchangeArithmetic::try_thb(foreign, ref_rate_parsed, unit_val).map_err(|e| {
        AppError::Internal(format!(
            "amount overflow in reference rate calculation: {e}"
        ))
    })?;
    let ref_principal = Decimal::from_i128_with_scale(ref_thb.minor_i128(), 2);
    let spread = if is_buy {
        ref_principal - principal
    } else {
        principal - ref_principal
    };
    Ok((ref_principal, spread))
}

/// Computes the reference spread percentage with 2 decimal places using kernel rounding (`round_cents` -> `fixed2`).
pub fn compute_reference_percent(spread: Decimal, total_ref_value: Decimal) -> AppResult<String> {
    if total_ref_value.is_zero() {
        return Ok("0.00".to_string());
    }
    let pct_cents =
        round_cents(spread * Decimal::from(100) / total_ref_value).ok_or_else(|| {
            AppError::Internal("reference percent overflow in reference rate calculation".into())
        })?;
    Ok(fixed2(pct_cents))
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
    shift_balances: Arc<dyn ShiftBalances>,
}

impl AdminCashBranchView {
    fn new(pool: PgPool, shift_balances: Arc<dyn ShiftBalances>) -> Self {
        Self {
            pool,
            shift_balances,
        }
    }
}

#[derive(sqlx::FromRow)]
struct TxRefRow {
    shop_side: String,
    thb_amount: Decimal,
    quantity: Decimal,
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
        let shift_balances = self.shift_balances.clone();
        let branch = branch.clone();
        let branch_id = branch.as_str().to_string();
        let show_all = filter.all;

        async move {
            let currencies = register_currencies();

            // 1. Last counted from each teller's newest closed shift at the branch
            let closed_ids: Vec<String> = sqlx::query_scalar(
                "SELECT DISTINCT ON (teller_id) id \
                 FROM shifts \
                 WHERE branch_id = $1 AND status = 'CLOSED' \
                 ORDER BY teller_id, seq DESC",
            )
            .bind(&branch_id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let last_counted = if !closed_ids.is_empty() {
                let mut counted_map: BTreeMap<String, Money> = BTreeMap::new();
                for closed_id in &closed_ids {
                    let closed_balances = shift_balances.shift_balances(closed_id).await?;
                    for r in closed_balances {
                        if let Some(c) = r.counted {
                            let code = r.currency.as_str().to_string();
                            let slot = counted_map.entry(code).or_insert(Money::ZERO);
                            *slot = checked_add_money(*slot, c)?;
                        }
                    }
                }

                let mut items = Vec::new();
                // THB always (even at zero)
                let thb_amount = counted_map.get("THB").copied().unwrap_or(Money::ZERO);
                items.push(AdminCashCountedItem {
                    code: "THB".to_string(),
                    amount: format!("{thb_amount}"),
                });

                // Other currencies counted > 0, sorted A-Z
                let mut other_codes: Vec<String> = counted_map
                    .iter()
                    .filter(|(code, amt)| code.as_str() != "THB" && !amt.is_zero())
                    .map(|(code, _)| code.clone())
                    .collect();
                other_codes.sort();

                for code in other_codes {
                    let amt = counted_map[&code];
                    items.push(AdminCashCountedItem {
                        code,
                        amount: format!("{amt}"),
                    });
                }

                Some(items)
            } else {
                None
            };

            // 2. Open shifts check
            let open_rows =
                xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                    pool.clone(),
                )
                .open_of_branch(&branch)
                .await?;

            if open_rows.is_empty() {
                return Ok(AdminCashData {
                    open_shifts: Vec::new(),
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
            }

            let shift_ids: Vec<String> = open_rows.iter().map(|r| r.shift_id.clone()).collect();
            let open_shifts: Vec<OpenShiftDto> =
                open_rows.into_iter().map(OpenShiftDto::from).collect();

            // 3. Stock rows summed across all OPEN shifts
            let mut balance_map: BTreeMap<String, ShiftBalance> = BTreeMap::new();
            for shift_id in &shift_ids {
                let open_balances = shift_balances.shift_balances(shift_id).await?;
                for b in open_balances {
                    let code = b.currency.as_str().to_string();
                    if let Some(acc) = balance_map.get_mut(&code) {
                        acc.opening = checked_add_money(acc.opening, b.opening)?;
                        acc.expected = acc.expected.checked_add(b.expected).map_err(|_| {
                            AppError::Internal("expected balance sum overflow".into())
                        })?;
                        acc.moved_in = checked_add_money(acc.moved_in, b.moved_in)?;
                        acc.moved_out = checked_add_money(acc.moved_out, b.moved_out)?;
                    } else {
                        balance_map.insert(code, b);
                    }
                }
            }

            // RE-7 #604 (ruling Q1 → B): the "rated" codes are the branch's current
            // publication's lines; while the branch has no publication, the currency
            // register, so the default list never empties before the first publication
            let current_publication: Option<uuid::Uuid> =
                sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
                    .bind(&branch_id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(AppError::Database)?
                    .flatten();
            let rated_codes: Vec<String> = match current_publication {
                Some(publication) => sqlx::query_scalar(
                    "SELECT DISTINCT trim(currency_code) FROM published_rate_lines \
                     WHERE publication_id = $1",
                )
                .bind(publication)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?,
                None => register_currencies(),
            };

            let mut codes_to_show = if show_all {
                register_currencies()
            } else {
                let mut candidates = BTreeSet::new();
                candidates.insert("THB".to_string());
                for c in rated_codes {
                    candidates.insert(c);
                }
                for (code, b) in &balance_map {
                    let net = b
                        .expected
                        .checked_minus(b.opening)
                        .map_err(|_| AppError::Internal("net balance overflow".into()))?;
                    if !b.opening.is_zero() || !b.expected.is_zero() || net != MoneyDelta::ZERO {
                        candidates.insert(code.clone());
                    }
                }
                sorted_currency_codes(candidates)
            };

            if let Some(ref c) = filter.currency {
                codes_to_show.retain(|code| code == c);
            }

            let mut stock = Vec::with_capacity(codes_to_show.len());
            for code in codes_to_show {
                let (opening, net, expected) = match balance_map.get(&code) {
                    Some(b) => {
                        let net = b
                            .expected
                            .checked_minus(b.opening)
                            .map_err(|_| AppError::Internal("net balance overflow".into()))?;
                        (
                            b.opening.to_string(),
                            net.to_string(),
                            b.expected.to_string(),
                        )
                    }
                    None => ("0.00".to_string(), "0.00".to_string(), "0.00".to_string()),
                };
                stock.push(AdminCashStockRow {
                    code,
                    opening,
                    net,
                    expected,
                });
            }

            // 4. Stats (cash flow in THB)
            let thb_balance = balance_map.get("THB");
            let thb_opening = thb_balance
                .map(|b| Decimal::from_i128_with_scale(b.opening.minor_i128(), 2))
                .unwrap_or(Decimal::ZERO);

            let cash_received: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(SUM(tl.settle_thb), 0) \
                 FROM transaction_lines tl \
                 JOIN transactions t ON t.id = tl.transaction_id \
                 JOIN transaction_payments tp ON tp.transaction_id = t.id \
                 WHERE t.shift_id = ANY($1) \
                   AND t.status = 'COMPLETED' \
                   AND tl.shop_side = 'SELL' \
                   AND tp.method = 'Cash'",
            )
            .bind(&shift_ids)
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;

            let thb_in = thb_balance
                .map(|b| Decimal::from_i128_with_scale(b.moved_in.minor_i128(), 2))
                .unwrap_or(Decimal::ZERO);

            let incoming = cash_received + thb_in;

            let cash_paid: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(SUM(tl.settle_thb), 0) \
                 FROM transaction_lines tl \
                 JOIN transactions t ON t.id = tl.transaction_id \
                 JOIN transaction_payments tp ON tp.transaction_id = t.id \
                 WHERE t.shift_id = ANY($1) \
                   AND t.status = 'COMPLETED' \
                   AND tl.shop_side = 'BUY' \
                   AND tp.method = 'Cash'",
            )
            .bind(&shift_ids)
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;

            let thb_out = thb_balance
                .map(|b| Decimal::from_i128_with_scale(b.moved_out.minor_i128(), 2))
                .unwrap_or(Decimal::ZERO);

            let outgoing = cash_paid + thb_out;
            let remaining = thb_opening + incoming - outgoing;

            let stats = Some(AdminCashStats {
                opening: format!("{:.2}", thb_opening),
                incoming: format!("{:.2}", incoming),
                outgoing: format!("{:.2}", outgoing),
                remaining: format!("{:.2}", remaining),
            });

            // 5. Reference summary over open shifts' exchanges
            let tx_rows: Vec<TxRefRow> = sqlx::query_as(
                "SELECT tl.shop_side, tl.thb_amount, tl.quantity, tl.reference_rate, tl.unit_size \
                 FROM transaction_lines tl \
                 JOIN transactions t ON t.id = tl.transaction_id \
                 WHERE t.shift_id = ANY($1) \
                   AND t.status = 'COMPLETED'",
            )
            .bind(&shift_ids)
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
                        tx.quantity,
                        ref_rate,
                        tx.unit_size,
                        tx.thb_amount,
                        tx.shop_side == "BUY",
                    )?;
                    total_spread += spread;
                    total_ref_value += ref_principal;
                } else {
                    without_reference += 1;
                }
            }

            let percent = compute_reference_percent(total_spread, total_ref_value)?;
            let spread_cents = round_cents(total_spread).ok_or_else(|| {
                AppError::Internal("spread overflow in reference rate calculation".into())
            })?;
            let reference = AdminCashReference {
                with_reference,
                without_reference,
                spread: fixed2(spread_cents),
                percent,
            };

            Ok(AdminCashData {
                open_shifts,
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
    ViewEndpoint::register(
        cfg,
        Arc::new(AdminCashBranchView::new(
            state.db.clone(),
            state.shift_balances.clone(),
        )),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminCashFilter::export_all(&cfg)?;
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
    fn test_positive_half_rounding() {
        // 2.525 rounds to 2.53 under kernel round_cents -> fixed2 (differs from half-even 2.52)
        let spread = Decimal::from_str("2.525").unwrap();
        let total_ref_value = Decimal::from(100);
        let result = compute_reference_percent(spread, total_ref_value).unwrap();
        assert_eq!(result, "2.53");
    }

    #[test]
    fn test_percent_two_decimal_places_formatting() {
        // 1.5% must format as "1.50", not "1.5" (goes red if {:.2} becomes to_string())
        let spread = Decimal::from_str("1.5").unwrap();
        let total_ref_value = Decimal::from(100);
        let result = compute_reference_percent(spread, total_ref_value).unwrap();
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
