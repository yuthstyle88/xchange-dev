//! The ledger's persistence port (design §8; LG-2 and LG-7 use it). Scaffold by the ticket author.

use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use super::account::{AccountKind, CashAccountId};
use super::posting::{Posting, PostingId};

pub trait LedgerRepository: Send + Sync {
    /// Finds or creates the account `(org, branch, currency, kind, shift)` inside `tx`
    /// (`INSERT … ON CONFLICT DO NOTHING RETURNING`, then `SELECT`; design §3.1).
    #[allow(clippy::too_many_arguments)]
    fn account_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: &'a str,
        branch: Option<&'a BranchId>,
        currency: CurrencyCode,
        kind: AccountKind,
        shift: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<CashAccountId>>;

    /// Inserts the posting and its entries inside `tx`. The same `(organization, source)` again
    /// answers the existing posting's id and writes nothing. Balance is checked at commit by the
    /// `ledger_posting_balanced` trigger.
    fn insert_posting_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        posting: &'a Posting,
    ) -> BoxFuture<'a, AppResult<PostingId>>;

    /// Σ `quantity_signed` of the account's entries.
    fn balance_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        account: CashAccountId,
    ) -> BoxFuture<'a, AppResult<Decimal>>;
}
