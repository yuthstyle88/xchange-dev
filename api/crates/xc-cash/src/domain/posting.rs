//! Posting port for cash movements: the final shape of design `ledger-inventory.md` §4.1 (LG-2 #924).

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, MoneyDelta};
use xc_shift::domain::shift::value_objects::balances::Balances;

pub use crate::domain::ledger::posting::{PostingId, SourceRef};

pub type ShiftId = String;
pub type TransferId = String;

/// #551's `transaction_lines.id`.
pub type TransactionLineId = Uuid;

/// AP-1's approval request id: the request `allow_negative` names (§5.1).
pub type ApprovalRequestId = xc_approval::domain::request::RequestId;

/// Where money is. `Customer` is an exchange's counterparty; `Bank` takes non-cash payments and the
/// external moves (§4.1). `Variance` is a branch's count and transfer differences (§4.4 rows 6 and 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CashPlace {
    Drawer(ShiftId),
    Safe(BranchId),
    InTransit(TransferId),
    Customer,
    Bank,
    Variance(BranchId),
}

/// One leg of a posting. Every request of one `post_in` call shares its `source`.
#[derive(Debug, Clone)]
pub struct PostingRequest {
    pub from: CashPlace,
    pub to: CashPlace,
    pub currency: CurrencyCode,
    /// THB is carried as a `ForeignAmount` with minor unit 2.
    pub quantity: ForeignAmount,
    pub source: SourceRef,
    /// #551's line, for cost freezing (§4.3).
    pub line: Option<TransactionLineId>,
    /// #552: value this leg at that line's frozen cost (§4.6).
    pub reverses: Option<TransactionLineId>,
}

/// One posting's requests moving `amounts` (currency, quantity) from `from` to `to`: one request per
/// currency with a positive quantity, all under `source`, so one `post_in` call holds an entry pair
/// per currency (#929 option A). The OPEN count, the hand-over and the v2 shift actions build their
/// moves here.
pub fn moves(
    from: &CashPlace,
    to: &CashPlace,
    amounts: &[(CurrencyCode, rust_decimal::Decimal)],
    source: &SourceRef,
) -> Vec<PostingRequest> {
    amounts
        .iter()
        .filter(|(_, q)| *q > rust_decimal::Decimal::ZERO)
        .map(|(currency, q)| PostingRequest {
            from: from.clone(),
            to: to.clone(),
            currency: *currency,
            quantity: ForeignAmount::from_decimal(*q),
            source: source.clone(),
            line: None,
            reverses: None,
        })
        .collect()
}

/// A drawer's or a transfer's differences (currency, place − expected) as one posting's requests:
/// a shortfall moves `place → VARIANCE`, an overage `VARIANCE → place` (§4.4 rows 6 and 7).
pub fn differences(
    place: &CashPlace,
    variance: &CashPlace,
    diffs: &[(CurrencyCode, rust_decimal::Decimal)],
    source: &SourceRef,
) -> Vec<PostingRequest> {
    let short: Vec<_> = diffs.iter().map(|(c, d)| (*c, -*d)).collect();
    let mut reqs = moves(place, variance, &short, source);
    reqs.extend(moves(variance, place, diffs, source));
    reqs
}

/// What a posting answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostingResult {
    pub posting_id: PostingId,
    /// One per request with a `line`.
    pub costs: Vec<LineCost>,
    /// One per request with `reverses` (§4.6).
    pub reversed: Vec<ReversedCost>,
}

/// The cost and margin frozen on an exchange line (§4.3). The margin is signed: a sale below cost
/// has a negative one (lane controller's ruling, issuecomment-6106564990).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineCost {
    pub line: TransactionLineId,
    pub cost_basis_thb: Money,
    pub realized_margin_thb: Option<MoneyDelta>,
}

/// A reversed leg valued at the original line's cost `C` (§4.6). `difference_thb` = moved − C.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReversedCost {
    pub reverses: TransactionLineId,
    pub cost_thb: Money,
    pub difference_thb: MoneyDelta,
}

pub trait CashPostings: Send + Sync {
    /// Posts `req` as one posting inside `tx`. `allow_negative` names the approved `NEGATIVE_STOCK`
    /// request that lets it take a balance below zero; `None` refuses with `stockInsufficient`.
    fn post_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        req: &'a [PostingRequest],
        allow_negative: Option<ApprovalRequestId>,
    ) -> BoxFuture<'a, AppResult<PostingResult>>;

    fn expected_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        place: &'a CashPlace,
    ) -> BoxFuture<'a, AppResult<Balances>>;

    /// `None` when `place` has never had an entry; otherwise one signed net per currency that ever
    /// had one, zeros included, sorted by currency code (§4.2.1, LG-6.r5 #1044).
    fn activity_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _place: &'a CashPlace,
    ) -> BoxFuture<'a, AppResult<Option<Vec<(CurrencyCode, MoneyDelta)>>>> {
        Box::pin(async move {
            Err(pos_error::AppError::Internal(
                "activity_in is not implemented by this CashPostings".into(),
            ))
        })
    }
}
