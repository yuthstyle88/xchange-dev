//! `ExchangeFeedReader` (design §6.4): one keyset page of `broker_exchange_feed`.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use uuid::Uuid;

use crate::domain::feed_cursor::FeedCursor;

/// The largest page (Broker §12.3 asks with `limit=200`).
pub const MAX_LIMIT: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedStatus {
    Completed,
    Reversed,
}

impl FeedStatus {
    /// `COMPLETED` / `REVERSED`, as stored and as sent.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "COMPLETED",
            Self::Reversed => "REVERSED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedItem {
    pub id: Uuid,
    pub customer_sub: Option<String>,
    pub phone: Option<String>,
    pub status: FeedStatus,
    pub value_satang: i64,
    pub reversed_satang: i64,
    pub completed_at: DateTime<Utc>,
    pub reversed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedPage {
    pub items: Vec<FeedItem>,
    /// The last item's cursor; `None` on an empty page (Broker §7: never echo `since`).
    pub next_cursor: Option<FeedCursor>,
}

pub trait ExchangeFeedReader: Send + Sync {
    /// Rows after `since` (all rows when `None`) ordered by `(updated_at, transaction_id)`, at most
    /// `limit`. `limit` outside `1..=MAX_LIMIT` is `Err(AppError::Validation(..))`.
    fn page(&self, since: Option<FeedCursor>, limit: u32) -> BoxFuture<'_, AppResult<FeedPage>>;
}
