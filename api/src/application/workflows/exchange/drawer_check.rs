//! Key-in drawer check (design `docs/design/blueprint/ledger-inventory.md` §5.1.3 "1. Key-in check"):
//! the currencies a quote or an exchange would take out of the teller's drawer, and the shortfall against
//! `LedgerCashPostings::expected_in(tx, &CashPlace::Drawer(shift))`.

use std::collections::BTreeMap;

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use xc_cash::domain::posting::{CashPlace, CashPostings};
use xc_cash::domain::shortfall::{ShortPlace, Shortfall, StockShortfall};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_exchange::domain::quote::quote_line::QuoteLine;
use xc_kernel::money::{CashStep, CurrencyCode, Settlement, SettlementError, Side};

use super::quote::CASH_STEP_NOT_SET;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payout {
    Cash,
    NonCash,
    Undecided,
}

/// What a quote's lines would take out of the drawer. `Payout::Undecided` estimates a cash payout, so it needs the
/// organisation's `thb_cash_step` (GA-4, row 5): `None` there is the `pricingRuleNotSet` / `thbCashStep` refusal.
pub fn quote_needs(
    lines: &[QuoteLine],
    payout: Payout,
    cash_step: Option<CashStep>,
) -> AppResult<Vec<(CurrencyCode, Decimal)>> {
    let mut out = Vec::new();
    for line in lines {
        match (line.shop_side, payout) {
            (Side::Buy, Payout::Cash) => {
                let settle = Decimal::from_i128_with_scale(line.settle_thb.minor_i128(), 2);
                out.push((CurrencyCode::THB, settle));
            }
            (Side::Buy, Payout::NonCash) => {}
            (Side::Buy, Payout::Undecided) => {
                let settle =
                    Settlement::of(Side::Buy, line.thb_amount, line.fee_thb, true, cash_step)
                        .map_err(|e| match e {
                            SettlementError::CashStepNotSet => {
                                AppError::Domain(CASH_STEP_NOT_SET.to_string())
                            }
                            other => AppError::Domain(other.to_string()),
                        })?
                        .settle();
                out.push((
                    CurrencyCode::THB,
                    Decimal::from_i128_with_scale(settle.minor_i128(), 2),
                ));
            }
            (Side::Sell, _) => {
                out.push((line.currency, line.quantity.value()));
            }
        }
    }
    Ok(out)
}

/// What one line needs from the drawer: a cash BUY needs `settle_thb` THB; a non-cash BUY needs
/// nothing; a SELL needs `quantity` of the currency.
pub fn needs(
    side: Side,
    currency: CurrencyCode,
    quantity: Decimal,
    settle_thb: Decimal,
    cash: bool,
) -> Vec<(CurrencyCode, Decimal)> {
    match side {
        Side::Buy if cash => vec![(CurrencyCode::THB, settle_thb)],
        Side::Buy => Vec::new(),
        Side::Sell => vec![(currency, quantity)],
    }
}

/// One `Shortfall { place: Drawer, short_by: need − have }` per currency whose need exceeds the
/// drawer's expected balance, sorted by currency code.
pub async fn drawer_shortfall_in(
    tx: &mut Transaction<'_, Postgres>,
    shift_id: &str,
    needs: &[(CurrencyCode, Decimal)],
) -> AppResult<StockShortfall> {
    if needs.is_empty() {
        return Ok(StockShortfall::default());
    }
    let mut totals: BTreeMap<&str, (CurrencyCode, Decimal)> = BTreeMap::new();
    for &(currency, amount) in needs {
        if amount > Decimal::ZERO {
            totals
                .entry(currency.as_str())
                .and_modify(|(_, sum)| *sum += amount)
                .or_insert((currency, amount));
        }
    }
    if totals.is_empty() {
        return Ok(StockShortfall::default());
    }
    let balances = LedgerCashPostings::new()
        .expected_in(tx, &CashPlace::Drawer(shift_id.to_owned()))
        .await?;
    let mut lines = Vec::new();
    for (_, (currency, need)) in totals {
        let have = balances.get(currency).value();
        if need > have {
            lines.push(Shortfall {
                currency,
                place: ShortPlace::Drawer,
                short_by: (need - have).normalize(),
            });
        }
    }
    Ok(StockShortfall { lines })
}
