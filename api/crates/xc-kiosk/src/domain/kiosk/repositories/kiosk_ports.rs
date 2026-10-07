//! What the kiosk asks of the other contexts.
//!
//! A context crate does not depend on another one, so the branch, its shift,
//! its counter rates, its payment rules and the shop's audit log reach the kiosk through this
//! trait. The root package implements it; the tests use a fake.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::payment::PaymentRules;

use crate::domain::kiosk::commands::save_counter_rate::SaveCounterRate;
use crate::domain::kiosk::value_objects::audit_line::AuditLine;
use crate::domain::kiosk::value_objects::counter_view::CounterView;

pub trait KioskPorts: Send + Sync {
    /// The branch's name; `None` when there is no such branch.
    fn branch_name<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Option<String>>>;

    /// The branch's counter as committed; `None` when there is no such branch.
    fn counter<'a>(&'a self, branch: &'a BranchId)
    -> BoxFuture<'a, AppResult<Option<CounterView>>>;

    /// The same, read inside the caller's transaction.
    fn counter_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<CounterView>>>;

    /// Saves a counter rate through the rate context, in a transaction of its
    /// own. A refusal is that context's sentence and is passed on unchanged.
    fn save_rate<'a>(
        &'a self,
        branch: &'a BranchId,
        rate: &'a SaveCounterRate,
        actor: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Writes one audit line inside the caller's transaction.
    fn record_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line: &'a AuditLine,
        at: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// The branch's payment rules as committed (design §5.5.2). The default answers v2's
    /// defaults, so a port that does not know the rules behaves as before they existed.
    fn payment_rules<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<PaymentRules>> {
        let _ = branch;
        Box::pin(async { Ok(PaymentRules::defaults()) })
    }

    /// The same, read inside the caller's transaction.
    fn payment_rules_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<PaymentRules>> {
        let _ = (tx, branch);
        Box::pin(async { Ok(PaymentRules::defaults()) })
    }
}
