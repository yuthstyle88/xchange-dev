//! `PgExchangeFeedReader`: `ExchangeFeedReader` on `broker_exchange_feed` (migration
//! `20261105100000_broker_exchange_feed.sql`), keyset on its `(updated_at, transaction_id)` index.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::feed_cursor::FeedCursor;
use crate::domain::feed_reader::{ExchangeFeedReader, FeedItem, FeedPage, FeedStatus, MAX_LIMIT};

pub struct PgExchangeFeedReader {
    pool: PgPool,
}

impl PgExchangeFeedReader {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct FeedRow {
    transaction_id: String,
    customer_sub: Option<String>,
    phone: Option<String>,
    status: String,
    value_satang: i64,
    reversed_satang: i64,
    completed_at: DateTime<Utc>,
    reversed_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl ExchangeFeedReader for PgExchangeFeedReader {
    fn page(&self, since: Option<FeedCursor>, limit: u32) -> BoxFuture<'_, AppResult<FeedPage>> {
        Box::pin(async move {
            if !(1..=MAX_LIMIT).contains(&limit) {
                return Err(AppError::Validation(format!(
                    "limit must be between 1 and {MAX_LIMIT}, got {limit}"
                )));
            }

            let rows: Vec<FeedRow> = match since {
                Some(cursor) => sqlx::query_as(
                    "SELECT transaction_id::text AS transaction_id, \
                                customer_sub, phone, status, value_satang, reversed_satang, \
                                completed_at, reversed_at, updated_at \
                         FROM broker_exchange_feed \
                         WHERE (updated_at, transaction_id) > ($1, $2::uuid) \
                         ORDER BY updated_at ASC, transaction_id ASC \
                         LIMIT $3",
                )
                .bind(cursor.updated_at)
                .bind(cursor.id.to_string())
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?,
                None => sqlx::query_as(
                    "SELECT transaction_id::text AS transaction_id, \
                                customer_sub, phone, status, value_satang, reversed_satang, \
                                completed_at, reversed_at, updated_at \
                         FROM broker_exchange_feed \
                         ORDER BY updated_at ASC, transaction_id ASC \
                         LIMIT $1",
                )
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?,
            };

            let mut items = Vec::with_capacity(rows.len());
            for row in rows {
                let id = Uuid::parse_str(&row.transaction_id).map_err(|e| {
                    AppError::Internal(format!("invalid transaction_id uuid in feed: {e}"))
                })?;
                let status = match row.status.as_str() {
                    "COMPLETED" => FeedStatus::Completed,
                    "REVERSED" => FeedStatus::Reversed,
                    other => {
                        return Err(AppError::Internal(format!(
                            "unknown feed status '{other}' in broker_exchange_feed"
                        )));
                    }
                };
                items.push(FeedItem {
                    id,
                    customer_sub: row.customer_sub,
                    phone: row.phone,
                    status,
                    value_satang: row.value_satang,
                    reversed_satang: row.reversed_satang,
                    completed_at: row.completed_at,
                    reversed_at: row.reversed_at,
                    updated_at: row.updated_at,
                });
            }

            let next_cursor = items.last().map(|last| FeedCursor {
                updated_at: last.updated_at,
                id: last.id,
            });

            Ok(FeedPage { items, next_cursor })
        })
    }
}
