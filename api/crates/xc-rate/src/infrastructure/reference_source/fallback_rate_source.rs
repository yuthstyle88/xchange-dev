//! `FallbackRateSource`: the primary's answer, or the fallback's when the primary fails
//! (design §8.7).
//!
//! SCAFFOLD for #1066's up-front tests: `fetch` is `todo!()`, because any shortcut answer
//! (primary only, fallback only) would already pass one row of the behaviour table.

use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::AppResult;

use crate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};

pub struct FallbackRateSource {
    primary: Arc<dyn RateSource>,
    fallback: Arc<dyn RateSource>,
}

impl FallbackRateSource {
    pub fn new(primary: Arc<dyn RateSource>, fallback: Arc<dyn RateSource>) -> Self {
        Self { primary, fallback }
    }
}

impl RateSource for FallbackRateSource {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        Box::pin(async move {
            match self.primary.fetch().await {
                Ok(rates) => Ok(rates),
                Err(_) => self.fallback.fetch().await,
            }
        })
    }
}
