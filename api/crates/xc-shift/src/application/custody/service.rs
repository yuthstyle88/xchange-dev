//! Custody application service.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::Actor;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::application::shift::service::ShiftService;
use crate::domain::custody::branch_day::BranchDay;
use crate::domain::custody::entities::custody_machine::CustodyMachine;
use crate::domain::custody::entities::customer_hold::HoldStatus;
use crate::domain::custody::entities::stored_event::StoredEvent;
use crate::domain::custody::repositories::custody_repository::CustodyRepository;
use crate::domain::custody::state::CustodyState;
use crate::domain::custody::value_objects::custody_actor::CustodyActor;
use crate::domain::custody::value_objects::custody_error::CustodyError;
use crate::domain::custody::value_objects::custody_facts::{
    CustodyFacts, OpenShift as FactOpenShift,
};
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::save_attempt::{PriorSave, SaveAttempt};
use crate::domain::custody::value_objects::service_window::ServiceWindow;
use crate::domain::shift::repositories::shift_repository::ShiftRepository;
use crate::domain::shift::value_objects::shift_error::ShiftError;
use crate::infrastructure::custody::custody_repository::PgCustodyRepository;
use crate::infrastructure::shift::shift_repository::PgShiftRepository;

pub trait CustodyFactsPort: Send + Sync + 'static {
    fn service_window_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        now: CustodyTime,
    ) -> BoxFuture<'a, AppResult<(ServiceWindow, i32)>>;

    fn has_live_rate_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<(bool, String)>>;

    fn transactions_summary_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<(Vec<String>, Option<String>)>>;

    fn customer_exists_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>>;
}

pub struct CustodyService<
    R: CustodyRepository = PgCustodyRepository,
    S: ShiftRepository = PgShiftRepository,
> {
    repo: Arc<R>,
    shifts: ShiftService<S>,
    facts: Arc<dyn CustodyFactsPort>,
    branch_tx: BranchTx,
}

impl<R: CustodyRepository, S: ShiftRepository> Clone for CustodyService<R, S> {
    fn clone(&self) -> Self {
        Self {
            repo: Arc::clone(&self.repo),
            shifts: self.shifts.clone(),
            facts: Arc::clone(&self.facts),
            branch_tx: self.branch_tx.clone(),
        }
    }
}

fn map_custody_error(err: CustodyError) -> AppError {
    AppError::Domain(err.to_string())
}

impl<R: CustodyRepository, S: ShiftRepository> CustodyService<R, S> {
    pub fn new(
        repo: Arc<R>,
        shifts: ShiftService<S>,
        facts: Arc<dyn CustodyFactsPort>,
        branch_tx: BranchTx,
    ) -> Self {
        Self {
            repo,
            shifts,
            facts,
            branch_tx,
        }
    }

    pub fn repo(&self) -> &Arc<R> {
        &self.repo
    }

    pub fn shifts(&self) -> &ShiftService<S> {
        &self.shifts
    }

    pub fn facts(&self) -> &Arc<dyn CustodyFactsPort> {
        &self.facts
    }

    pub fn branch_tx(&self) -> &BranchTx {
        &self.branch_tx
    }

    pub async fn state(&self, branch: &BranchId) -> AppResult<CustodyState> {
        self.repo.load_state(branch).await
    }

    pub async fn events(&self, branch: &BranchId) -> AppResult<Vec<StoredEvent>> {
        self.repo.list_events(branch).await
    }

    pub async fn check_save_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        save: &SaveAttempt,
        actor: &CustodyActor,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let state = self.repo.load_state_in(tx, branch).await?;
        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());
        let (service_window, utc_offset_minutes) = self
            .facts
            .service_window_in(tx, branch, custody_now)
            .await?;
        let active_shift = match actor
            .employee_id
            .as_deref()
            .filter(|s| uuid::Uuid::parse_str(s).is_ok())
        {
            Some(emp) => self.shifts.open_of_teller_in(tx, emp).await?,
            None => None,
        };
        let open_shift = active_shift.map(|s| FactOpenShift {
            id: s.id,
            expected: s.expected,
        });
        let (has_live_rate, rates_updated) = self.facts.has_live_rate_in(tx, branch).await?;
        let (pending_transactions, last_transaction) =
            self.facts.transactions_summary_in(tx, branch).await?;
        let customer_known = self
            .facts
            .customer_exists_in(tx, branch, &save.customer_id)
            .await?;

        let facts = CustodyFacts {
            service_window,
            utc_offset_minutes,
            open_shift,
            has_live_rate,
            rates_updated,
            pending_transactions,
            last_transaction,
            customer_known,
            new_id: String::new(),
        };

        CustodyMachine::check_save(&state, save, actor, &facts, custody_now)
            .map_err(map_custody_error)
    }

    /// CA-7 (#623, `cash-shift-transfer.md` §4.3): the save gate for a Blueprint-session teller (`C1`).
    /// Looks up the teller's own OPEN shift in `branch` by `shifts.teller_id = saver.user_id::uuid`;
    /// without one the answer is the save's own `noOpenShift`.
    pub async fn check_teller_save_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        save: &SaveAttempt,
        saver: &Actor,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let state = self.repo.load_state_in(tx, branch).await?;

        // Step 1: customer on hold (same as v2).
        if state
            .holds
            .iter()
            .any(|h| h.customer_id == save.customer_id && h.status == HoldStatus::Open)
        {
            return Err(map_custody_error(CustodyError::CustomerOnHold));
        }

        // Step 2: service hours (only on a fresh save, same as v2).
        let custody_now = CustodyTime::from_epoch_millis(now.timestamp_millis());
        let (service_window, _) = self
            .facts
            .service_window_in(tx, branch, custody_now)
            .await?;
        if save.prior == PriorSave::None && !service_window.allows_save(custody_now) {
            return Err(map_custody_error(CustodyError::SaveClosed));
        }

        // Step 3: teller's own OPEN shift on this branch (keyed by shifts.teller_id).
        let own_shift: Option<String> = match uuid::Uuid::parse_str(&saver.user_id) {
            Ok(teller_uuid) => {
                sqlx::query_scalar(
                    "SELECT id FROM shifts \
                     WHERE branch_id = $1 AND teller_id = $2 AND status = 'OPEN'",
                )
                .bind(branch.as_str())
                .bind(teller_uuid)
                .fetch_optional(&mut **tx)
                .await?
            }
            Err(_) => None,
        };

        let open_shifts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM shifts WHERE branch_id = $1 AND status = 'OPEN'",
        )
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await?;
        let phase = BranchDay::read_phase(state.phase, usize::try_from(open_shifts).unwrap_or(0));

        let window_needs_draft = service_window.needs_draft(custody_now);
        BranchDay::check_save(
            phase,
            saver,
            own_shift.as_deref(),
            &state.drafts,
            save,
            window_needs_draft,
        )
        .map_err(|err| match err {
            CustodyError::NoOpenShift => ShiftError::NoOpenShift.into(),
            other => map_custody_error(other),
        })
    }

    pub async fn complete_draft_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        save: &SaveAttempt,
        document_ref: &str,
    ) -> AppResult<()> {
        let state = self.repo.load_state_in(tx, branch).await?;
        let new_state = CustodyMachine::complete_draft(&state, save, document_ref);
        self.repo
            .store_state_in(tx, branch, &state, &new_state)
            .await
    }
}
