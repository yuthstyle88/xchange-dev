//! Reference rate source port.

use futures::future::BoxFuture;
use pos_error::AppResult;

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceRates {
    pub source: String,
    pub fetched_at: String,
    pub kind: String,
    pub rows: Vec<serde_json::Value>,
}

pub trait RateSource: Send + Sync {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>>;
}
