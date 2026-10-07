//! What the employee screen needs from custody.

use std::future::Future;
use std::pin::Pin;

use pos_error::AppResult;
use serde_json::Value;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_kernel::ids::BranchId;

/// What the employee screen needs from custody. Looked up as `web::Data<dyn EmployeeCustody>`;
/// `configure` registers `PgEmployeeCustody`. In an app without one nobody holds cash and the
/// two custody actions are bad requests.
pub trait EmployeeCustody: Send + Sync {
    fn holds_cash_or_pending<'a>(
        &'a self,
        branch: &'a BranchId,
        employee_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<bool>> + 'a>>;
    /// The same question, read through `tx` — the transaction that holds the branch's lock
    /// while `disable` closes the account (`DisableEmployee`). The future is `Send` because it
    /// runs inside `BranchTx::run`. By default `false`: a port with no transaction to ask in
    /// has answered on the pool already, by `holds_cash_or_pending`.
    fn holds_cash_or_pending_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        employee_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<bool>> + Send + 'a>> {
        let _ = (tx, branch, employee_id);
        Box::pin(async { Ok(false) })
    }
    fn set_entity<'a>(
        &'a self,
        payload: &'a Value,
        actor: &'a Actor,
    ) -> Pin<Box<dyn Future<Output = AppResult<Value>> + 'a>>;
    fn enable<'a>(
        &'a self,
        payload: &'a Value,
        actor: &'a Actor,
    ) -> Pin<Box<dyn Future<Output = AppResult<Value>> + 'a>>;
}
