//! `PgEmployeeCustody`: what the owner's employee screen needs from custody.
//!
//! `/api/employees` is the only door the old server gave the owner to custody:
//! its actions `entity` and `enable` are the custody commands `opsSetEntity` and
//! `opsEnable`, and its `disable` first asks custody whether the employee still
//! holds cash or has work pending. The route declares those three questions as
//! the trait `EmployeeCustody`; this is the implementation, on top of the custody
//! service. It holds no SQL of its own: it reads through the service's
//! repository and writes through `CustodyService::command`, which runs inside
//! one `BranchTx`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde_json::Value;
use sqlx::PgPool;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_actor::CustodyActor;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::request_status::RequestStatus;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::infrastructure::custody::custody_ports::PgCustodyPorts;
use crate::presentation::http::employees::EmployeeCustody;

/// The old server's sentence for a request it cannot read.
const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
/// The branch a payload means when it names none.
const DEFAULT_BRANCH: &str = "MAIN";

/// Answers `EmployeeCustody` from the custody service, and from the custody
/// repository it is built on for the question asked inside another transaction.
#[derive(Clone)]
pub struct PgEmployeeCustody {
    custody: CustodyService,
    repo: Arc<PgCustodyRepository>,
}

impl PgEmployeeCustody {
    /// The custody service on this pool, with the real ports, and its repository.
    pub fn new(pool: PgPool) -> Self {
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        let shifts = ShiftService::new(
            Arc::new(PgShiftRepository::new(pool.clone())),
            branch_tx.clone(),
        );
        let repo = Arc::new(PgCustodyRepository::new(pool));
        Self {
            custody: CustodyService::new(repo.clone(), shifts, Arc::new(PgCustodyPorts), branch_tx),
            repo,
        }
    }

    async fn command(&self, action: &str, payload: &Value, actor: &Actor) -> AppResult<Value> {
        let branch = branch_of(payload)?;
        self.custody
            .command(
                &branch,
                action,
                payload,
                &custody_actor(actor),
                SystemTime::now().into(),
            )
            .await
    }
}

/// The branch a custody payload is about: `branchId`, or `MAIN` when the payload
/// names none (the recorded `entity` of the owner carries no `branchId` and lands
/// in `MAIN`). Anything else than 1–70 letters, digits and hyphens is a bad
/// request — the old server's answer for that was not recorded.
fn branch_of(payload: &Value) -> AppResult<BranchId> {
    match payload.get("branchId") {
        None | Some(Value::Null) => Ok(BranchId::new(DEFAULT_BRANCH)),
        Some(Value::String(id)) if BranchId::is_well_formed(id) => Ok(BranchId::new(id.as_str())),
        Some(_) => Err(AppError::Domain(BAD_REQUEST.to_owned())),
    }
}

/// Who custody sees: the actor's id, never a name. Only the owner is `Owner`;
/// the route lets nobody else in, and an employee who got here anyway is refused
/// by the custody rules as an employee with no part in switching custody on.
fn custody_actor(actor: &Actor) -> CustodyActor {
    match actor.kind() {
        ActorKind::Owner => CustodyActor {
            id: actor.id().to_owned(),
            employee_id: None,
            role: CustodyRole::Owner,
        },
        ActorKind::Staff => CustodyActor {
            id: actor.id().to_owned(),
            employee_id: Some(actor.id().to_owned()),
            role: CustodyRole::Other,
        },
    }
}

/// The old server's rule, measured on its recorded answers to `disable`
/// (`tests/fixtures/auth/v2_auth_http_cases.json`, story "employees"): the
/// employee is the branch's custodian, or the sender or the receiver of a request
/// that is still pending. It is the rule that keeps an employee from clocking out,
/// `xc-shift` `domain/custody/entities/custody_machine.rs:216-220`, which is not
/// public — keep the two alike.
fn holds(state: &CustodyState, employee_id: &str) -> bool {
    state.custodian.as_deref() == Some(employee_id)
        || state.requests.iter().any(|r| {
            r.status == RequestStatus::Pending
                && (r.sender == employee_id || r.receiver.as_deref() == Some(employee_id))
        })
}

impl EmployeeCustody for PgEmployeeCustody {
    fn holds_cash_or_pending<'a>(
        &'a self,
        branch: &'a BranchId,
        employee_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<bool>> + 'a>> {
        Box::pin(async move { Ok(holds(&self.custody.state(branch).await?, employee_id)) })
    }

    fn holds_cash_or_pending_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        employee_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<bool>> + Send + 'a>> {
        Box::pin(async move {
            let state = self.repo.load_state_in(tx, branch).await?;
            Ok(holds(&state, employee_id))
        })
    }

    fn set_entity<'a>(
        &'a self,
        payload: &'a Value,
        actor: &'a Actor,
    ) -> Pin<Box<dyn Future<Output = AppResult<Value>> + 'a>> {
        Box::pin(self.command("opsSetEntity", payload, actor))
    }

    fn enable<'a>(
        &'a self,
        payload: &'a Value,
        actor: &'a Actor,
    ) -> Pin<Box<dyn Future<Output = AppResult<Value>> + 'a>> {
        Box::pin(self.command("opsEnable", payload, actor))
    }
}
