//! Quote repository trait.

use pos_error::AppResult;
use uuid::Uuid;

use super::quote::Quote;

pub trait QuoteRepository: Send + Sync {
    fn insert_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        quote: &'a Quote,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    fn find_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
    ) -> impl std::future::Future<Output = AppResult<Option<Quote>>> + Send + 'a;
}
