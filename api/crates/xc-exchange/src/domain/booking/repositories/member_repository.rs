//! Member repository trait.

use futures::future::BoxFuture;
use pos_error::AppResult;
use thiserror::Error;
use xc_kernel::ids::BranchId;

use crate::domain::booking::entities::member::Member;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("token hash already taken")]
pub struct TokenTaken;

pub trait MemberRepository: Send + Sync {
    fn list<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Vec<Member>>>;

    fn find<'a>(&'a self, id: &'a str) -> BoxFuture<'a, AppResult<Option<Member>>>;

    fn find_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Member>>>;

    fn by_customer_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Member>>>;

    fn upsert_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        member: &'a Member,
    ) -> BoxFuture<'a, AppResult<Result<Member, TokenTaken>>>;
}
