//! PostgreSQL implementation of [`CurrencyRegister`].

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::money::{CurrencyCode, CurrencyUnits, MinorUnit, UnitSize};

use crate::domain::currency::currency_register::CurrencyRegister;

pub struct PgCurrencyRegister {
    pool: PgPool,
}

impl PgCurrencyRegister {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CurrencyRegister for PgCurrencyRegister {
    fn units(&self, code: CurrencyCode) -> BoxFuture<'_, AppResult<Option<CurrencyUnits>>> {
        Box::pin(async move {
            let row: Option<(Decimal, i16)> =
                sqlx::query_as("SELECT unit_size, minor_unit FROM currencies WHERE code = $1")
                    .bind(code.as_str())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::Internal(format!("fetch currency units for {code}: {e}"))
                    })?;

            match row {
                Some((unit_size_dec, minor_unit_i16)) => {
                    let unit_size = UnitSize::from_decimal(unit_size_dec).map_err(|e| {
                        AppError::Internal(format!("invalid unit_size for {code}: {e}"))
                    })?;
                    let minor_unit = MinorUnit::new(minor_unit_i16 as u8).map_err(|e| {
                        AppError::Internal(format!("invalid minor_unit for {code}: {e}"))
                    })?;
                    Ok(Some(CurrencyUnits::new(code, unit_size, minor_unit)))
                }
                None => Ok(None),
            }
        })
    }
}
