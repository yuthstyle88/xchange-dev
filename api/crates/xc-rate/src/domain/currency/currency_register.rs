//! Currency register port.

use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::money::{CurrencyCode, CurrencyUnits};

pub trait CurrencyRegister: Send + Sync {
    fn units(&self, code: CurrencyCode) -> BoxFuture<'_, AppResult<Option<CurrencyUnits>>>;
}
