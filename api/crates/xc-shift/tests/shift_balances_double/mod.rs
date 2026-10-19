//! In-memory `ShiftBalances` test double for `xc-shift` integration tests (#1043, #3138).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};
use xc_shift::domain::shift::repositories::shift_balances::{ShiftBalance, ShiftBalances, ShiftId};
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;

fn parse_code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text.trim()).unwrap_or_else(|_| panic!("{text} is not in the register"))
}

fn parse_money(text: &str) -> Money {
    Money::parse(text.trim()).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn parse_delta(text: &str) -> MoneyDelta {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix('-') {
        MoneyDelta::ZERO.minus(parse_money(rest))
    } else {
        MoneyDelta::ZERO.plus(parse_money(trimmed))
    }
}

fn sort_rows(rows: &mut [ShiftBalance]) {
    rows.sort_by(|a, b| {
        let a_thb = a.currency == CurrencyCode::THB;
        let b_thb = b.currency == CurrencyCode::THB;
        b_thb
            .cmp(&a_thb)
            .then_with(|| a.currency.as_str().cmp(b.currency.as_str()))
    });
}

#[derive(Clone, Default)]
pub struct ShiftBalancesDouble {
    by_shift: Arc<Mutex<BTreeMap<ShiftId, Vec<ShiftBalance>>>>,
}

impl ShiftBalancesDouble {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed_opening(&self, shift_id: &str, opening: &[(&str, &str)]) {
        let mut guard = self.by_shift.lock().expect("ShiftBalancesDouble lock");
        let rows = guard.entry(shift_id.to_owned()).or_default();
        for (code, amount) in opening {
            let currency = parse_code(code);
            let m = parse_money(amount);
            let exp = MoneyDelta::from_minor(m.minor_i128());
            if let Some(existing) = rows.iter_mut().find(|r| r.currency == currency) {
                existing.opening = m;
                existing.expected = exp;
            } else {
                rows.push(ShiftBalance {
                    currency,
                    opening: m,
                    expected: exp,
                    moved_in: Money::ZERO,
                    moved_out: Money::ZERO,
                    counted: None,
                    difference: None,
                });
            }
        }
        sort_rows(rows);
    }

    pub fn seed_movement(&self, shift_id: &str, code: &str, direction: &str, amount: &str) {
        let mut guard = self.by_shift.lock().expect("ShiftBalancesDouble lock");
        let rows = guard.entry(shift_id.to_owned()).or_default();
        let currency = parse_code(code);
        let m = parse_money(amount);
        let signed = if direction.trim() == "IN" {
            MoneyDelta::ZERO.plus(m)
        } else {
            MoneyDelta::ZERO.minus(m)
        };
        let row = if let Some(existing) = rows.iter_mut().find(|r| r.currency == currency) {
            existing
        } else {
            rows.push(ShiftBalance {
                currency,
                opening: Money::ZERO,
                expected: MoneyDelta::ZERO,
                moved_in: Money::ZERO,
                moved_out: Money::ZERO,
                counted: None,
                difference: None,
            });
            rows.last_mut().expect("just pushed")
        };
        row.expected = MoneyDelta::from_minor(row.expected.minor() + signed.minor());
        if direction.trim() == "IN" {
            row.moved_in =
                Money::try_from_minor_i128(row.moved_in.minor_i128() + m.minor_i128()).unwrap();
        } else {
            row.moved_out =
                Money::try_from_minor_i128(row.moved_out.minor_i128() + m.minor_i128()).unwrap();
        }
        sort_rows(rows);
    }

    pub fn seed_close(&self, shift_id: &str, counted: &[(&str, &str)]) {
        let mut guard = self.by_shift.lock().expect("ShiftBalancesDouble lock");
        let rows = guard.entry(shift_id.to_owned()).or_default();
        for (code, amount) in counted {
            let currency = parse_code(code);
            let c = parse_money(amount);
            if let Some(existing) = rows.iter_mut().find(|r| r.currency == currency) {
                existing.counted = Some(c);
                existing.difference = Some(MoneyDelta::from_minor(
                    c.minor_i128() - existing.expected.minor(),
                ));
            } else {
                rows.push(ShiftBalance {
                    currency,
                    opening: Money::ZERO,
                    expected: MoneyDelta::ZERO,
                    moved_in: Money::ZERO,
                    moved_out: Money::ZERO,
                    counted: Some(c),
                    difference: Some(MoneyDelta::from_minor(c.minor_i128())),
                });
            }
        }
        sort_rows(rows);
    }

    pub fn apply_delta(&self, shift_id: &str, delta: &StockDelta) {
        let mut guard = self.by_shift.lock().expect("ShiftBalancesDouble lock");
        let rows = guard.entry(shift_id.to_owned()).or_default();
        for (currency, d) in delta.non_zero() {
            if let Some(existing) = rows.iter_mut().find(|r| r.currency == currency) {
                existing.expected = MoneyDelta::from_minor(existing.expected.minor() + d.minor());
            } else {
                rows.push(ShiftBalance {
                    currency,
                    opening: Money::ZERO,
                    expected: d,
                    moved_in: Money::ZERO,
                    moved_out: Money::ZERO,
                    counted: None,
                    difference: None,
                });
            }
        }
        sort_rows(rows);
    }

    pub fn seed_row(
        &self,
        shift_id: &str,
        code: &str,
        opening: &str,
        expected: &str,
        counted: Option<&str>,
        difference: Option<&str>,
    ) {
        let mut guard = self.by_shift.lock().expect("ShiftBalancesDouble lock");
        let rows = guard.entry(shift_id.to_owned()).or_default();
        let currency = parse_code(code);
        let row = ShiftBalance {
            currency,
            opening: parse_money(opening),
            expected: parse_delta(expected),
            moved_in: Money::ZERO,
            moved_out: Money::ZERO,
            counted: counted.map(parse_money),
            difference: difference.map(parse_delta),
        };
        if let Some(existing) = rows.iter_mut().find(|r| r.currency == currency) {
            *existing = row;
        } else {
            rows.push(row);
        }
        sort_rows(rows);
    }
}

impl ShiftBalances for ShiftBalancesDouble {
    fn shift_balances<'a>(
        &'a self,
        shift: &'a ShiftId,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftBalance>>> {
        let rows = self
            .by_shift
            .lock()
            .expect("ShiftBalancesDouble lock")
            .get(shift)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(rows) })
    }

    fn shift_balances_many<'a>(
        &'a self,
        shifts: &'a [ShiftId],
    ) -> BoxFuture<'a, AppResult<BTreeMap<ShiftId, Vec<ShiftBalance>>>> {
        let guard = self.by_shift.lock().expect("ShiftBalancesDouble lock");
        let mut out = BTreeMap::new();
        for shift in shifts {
            out.insert(shift.clone(), guard.get(shift).cloned().unwrap_or_default());
        }
        Box::pin(async move { Ok(out) })
    }
}
