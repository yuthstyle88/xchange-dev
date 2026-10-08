//! SnapshotSources: what the snapshot reads from contexts that have not landed.
//!
//! The snapshot builder reads branches, rates, customers, shifts, movements and
//! custody through their own services. Three things it shows have no service it
//! can call yet, so it asks for them through [`SnapshotSources`]:
//!
//! - the saved exchanges of a branch (the transactions table and its service
//!   are the save workflow's);
//! - which customers are members (decision D5, not taken);
//! - the names and account roles of the people of a branch, for the names v2
//!   kept inside custody rows.
//!
//! [`NothingYet`] answers all three with nothing. The real implementation
//! belongs under `infrastructure/` and is a thin adapter: every type here is
//! plain data shaped after what those services hand out.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Money, Rate, Side};
use xc_kernel::payment::PaymentRules;

use crate::application::compat::v2_json::People;

/// What a source answers: a boxed future, so the builder can hold
/// `Arc<dyn SnapshotSources>`.
pub type SourceFuture<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

/// One saved exchange as the snapshot shows it. The fields are those of the
/// save workflow's `Transaction` that v2 printed; nothing else is carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotTransaction {
    /// The document reference (`XC-20261006-000001`): v2's `id`.
    pub document_ref: String,
    /// When the exchange was recorded: v2's `created`.
    pub recorded_at: DateTime<Utc>,
    /// The value date, already a Bangkok calendar day: v2's `date`.
    pub value_date: NaiveDate,
    pub side: Side,
    /// The foreign currency's code.
    pub currency: String,
    pub amount: Money,
    pub rate: Rate,
    pub reference_rate: Option<Rate>,
    pub fee: Money,
    pub bank_fee: Money,
    /// The payment method as stored (`Cash`, `Bank Transfer`, `SCB Transfer`, …).
    /// The builder prints the old name `SCB Transfer` as `Bank Transfer`, as v2 did.
    pub method: String,
    /// The file key of the payment slip, when there is one.
    pub slip_key: Option<String>,
    pub payment_ref: String,
    pub customer_id: String,
    /// The customer's name as it was when the exchange was saved.
    pub customer_name: String,
    /// The username of who served the customer.
    pub staff: String,
    pub notes: String,
    /// The shift the exchange belongs to; always set for a LIVE exchange.
    pub shift_id: Option<String>,
}

/// A customer who is a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberMark {
    pub customer_id: String,
    /// When the membership was verified: v2's `memberVerifiedAt`.
    pub verified_at: Option<DateTime<Utc>>,
}

/// What the snapshot shows that no service on its side of the code can give yet.
pub trait SnapshotSources: Send + Sync {
    /// The LIVE exchanges of `branch`, oldest first. A DEMO exchange is never
    /// part of the answer: no snapshot shows one.
    fn transactions<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> SourceFuture<'a, Vec<SnapshotTransaction>>;

    /// The customers of `branch` who are members, in any order.
    fn members<'a>(&'a self, branch: &'a BranchId) -> SourceFuture<'a, Vec<MemberMark>>;

    /// Every person of `branch` by employee id — disabled accounts included —
    /// with the display name and the account's role as v2 spells it.
    fn people<'a>(&'a self, branch: &'a BranchId) -> SourceFuture<'a, People>;

    /// The payment rules of `branch`. Default answers `PaymentRules::defaults()`.
    fn payment_rules<'a>(&'a self, _branch: &'a BranchId) -> SourceFuture<'a, PaymentRules> {
        Box::pin(async { Ok(PaymentRules::defaults()) })
    }
}

/// The sources before any of the three has landed: no exchange, no member,
/// nobody.
#[derive(Debug, Clone, Copy, Default)]
pub struct NothingYet;

impl SnapshotSources for NothingYet {
    fn transactions<'a>(
        &'a self,
        _branch: &'a BranchId,
    ) -> SourceFuture<'a, Vec<SnapshotTransaction>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn members<'a>(&'a self, _branch: &'a BranchId) -> SourceFuture<'a, Vec<MemberMark>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn people<'a>(&'a self, _branch: &'a BranchId) -> SourceFuture<'a, People> {
        Box::pin(async { Ok(People::new()) })
    }
}
