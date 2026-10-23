//! One presentation helper for v3 inputs (design `docs/design/blueprint/bounded-inputs.md` §2 B-3, B-7, §3 `:72-78`;
//! tracking #2054). Each function maps a refusal to the house answer, 400 `validationFailed` with the field name.

use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use xc_kernel::money::{Bound, IntBound};

use super::command::server_error;
use super::error::V3Error;
use super::org_view::validation_failed;

/// A table a value must name a row of. The SQL is fixed per variant (no string-built table names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowRef {
    /// `currencies.code`.
    Currency,
}

/// Parses `text` under `bound`, mapping any [`xc_kernel::money::BoundError`] to `400 validationFailed` on `field`.
/// Never trims surrounding whitespace (O-P1).
pub fn decimal(field: &'static str, text: &str, bound: Bound) -> Result<Decimal, V3Error> {
    bound.parse(text).map_err(|_| validation_failed(field))
}

/// Returns the matching `&'static str` from `allowed` on an exact, case-sensitive match, or `400 validationFailed` on
/// `field`.
pub fn one_of(
    field: &'static str,
    text: &str,
    allowed: &[&'static str],
) -> Result<&'static str, V3Error> {
    allowed
        .iter()
        .copied()
        .find(|&candidate| candidate == text)
        .ok_or_else(|| validation_failed(field))
}

/// Checks `v` against `bound`, mapping any [`xc_kernel::money::BoundError`] to `400 validationFailed` on `field`.
pub fn integer(field: &'static str, v: i64, bound: IntBound) -> Result<i64, V3Error> {
    bound.check(v).map_err(|_| validation_failed(field))
}

/// Inclusive character-count bound (`min ..= max`) measured after `trim()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextBound {
    pub min: usize,
    pub max: usize,
}

/// Checks that `text.trim().chars().count()` lies in `bound.min ..= bound.max`, returning `400 validationFailed` on
/// `field` when outside.
pub fn text<'a>(field: &'static str, text: &'a str, bound: TextBound) -> Result<&'a str, V3Error> {
    let trimmed = text.trim();
    let len = trimmed.chars().count();
    if len < bound.min || len > bound.max {
        return Err(validation_failed(field));
    }
    Ok(trimmed)
}

/// Checks inside `tx` that `value` names an existing row of `table_key`, returning `400 validationFailed` on `field`
/// when missing or `500 serverError` on a database error.
pub async fn must_exist(
    tx: &mut Transaction<'_, Postgres>,
    field: &'static str,
    table_key: RowRef,
    value: &str,
) -> Result<(), V3Error> {
    let exists = match table_key {
        RowRef::Currency => {
            sqlx::query_scalar::<_, i32>("SELECT 1 FROM currencies WHERE code = $1")
                .bind(value)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|_| server_error())?
                .is_some()
        }
    };
    if exists {
        Ok(())
    } else {
        Err(validation_failed(field))
    }
}
