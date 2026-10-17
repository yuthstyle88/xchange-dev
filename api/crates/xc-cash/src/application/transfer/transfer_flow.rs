//! `TransferFlow`: orchestrates cash transfers across the 5 states (D:§4.5, §9.1 CA-5).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Target};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
use xc_shift::infrastructure::custody::custody_repository::append_cash_exception_in;

use crate::application::approval::{TransferApprovalPolicy, TransferApprovalSubject, policy_actor};
use crate::domain::ledger::posting::{SourceRef, SourceType};
use crate::domain::posting::{CashPlace, CashPostings, differences};
use crate::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferError, TransferStatus, TransferStep,
    add_line_minor,
};
use crate::domain::transfer::kinds::TransferKinds;
use crate::domain::transfer::transfer_repository::TransferRepository;
use crate::infrastructure::ledger_postings::LedgerCashPostings;
use crate::infrastructure::transfer_repository::PgTransferRepository;

pub struct TransferFlow {
    pool: PgPool,
    /// Dispatch, receive and reconcile run in a branch's `BranchTx` (design §09 atomicity,
    /// `cash-shift-transfer.md:517`): the posting and the state change commit together, under that
    /// branch's lock, and the branch version moves.
    branch_tx: BranchTx,
    repo: Arc<dyn TransferRepository>,
    postings: Arc<dyn CashPostings>,
    kinds: TransferKinds,
    approvals: Arc<ApprovalService>,
}

/// `who` holds `permission` on `branch` at `now` (D:§3.1), or the refusal's code as a domain error.
fn require(
    who: &Actor,
    permission: Permission,
    branch: &BranchId,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let target = Target::new(&who.organization_id, Some(branch.clone()), None);
    Authorizer::check(Some(who), permission, &target, now)
        .map(|_| ())
        .map_err(|e| AppError::Domain(e.code().to_string()))
}

fn from_transfer_error(e: TransferError) -> AppError {
    match e {
        TransferError::Internal(m) => AppError::Internal(m),
        other => AppError::Domain(other.to_string()),
    }
}

impl TransferFlow {
    pub fn new(
        pool: PgPool,
        repo: Arc<dyn TransferRepository>,
        postings: Arc<dyn CashPostings>,
        kinds: TransferKinds,
        approvals: Arc<ApprovalService>,
    ) -> Self {
        Self {
            branch_tx: BranchTx::new(Transactor::new(pool.clone())),
            pool,
            repo,
            postings,
            kinds,
            approvals,
        }
    }

    pub fn pg(pool: PgPool) -> Self {
        let repo = Arc::new(PgTransferRepository::new(pool.clone()));
        let postings = Arc::new(LedgerCashPostings::new());
        let kinds = TransferKinds::standard();

        let mut registry = ApprovalRegistry::new();
        registry.register(
            Arc::new(TransferApprovalSubject::new(pool.clone())),
            Arc::new(TransferApprovalPolicy::new()),
        );

        let approvals = Arc::new(ApprovalService::new(
            registry,
            Arc::new(PgApprovalRepository::new(pool.clone())),
            Arc::new(PgAuditLog::new(pool.clone())),
        ));

        Self::new(pool, repo, postings, kinds, approvals)
    }

    /// Creates a transfer in DRAFT status.
    pub async fn create(
        &self,
        who: &Actor,
        cmd: NewTransfer,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let mut tx = self.pool.begin().await?;
        let transfer = self.create_in(&mut tx, who, cmd, now).await?;
        tx.commit().await?;
        Ok(transfer)
    }

    /// Submits a transfer in DRAFT status to the approval core.
    pub async fn submit(
        &self,
        who: &Actor,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let mut tx = self.pool.begin().await?;
        let transfer = self.submit_in(&mut tx, who, id, now).await?;
        tx.commit().await?;
        Ok(transfer)
    }

    /// Decides on an approval request.
    pub async fn decide(
        &self,
        who: &Actor,
        id: &str,
        outcome: Outcome,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let t = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "CashTransfer",
                id: id.to_string(),
            })?;

        if t.status != TransferStatus::Draft {
            return Err(AppError::Domain(
                TransferError::IllegalTransition {
                    from: t.status,
                    step: TransferStep::Approve,
                }
                .to_string(),
            ));
        }

        if t.created_by == who.user_id {
            return Err(AppError::Domain(
                "creator cannot approve own transfer".into(),
            ));
        }

        require(who, Permission::ApprovalsDecide, &t.from_branch, now)?;
        // Who decides within and above the limit is `TransferApprovalPolicy::may_decide`'s (A5, ruling Q2),
        // over the limit recorded at submit; an approver that maps to no policy actor is refused here.
        let policy_actor = policy_actor(who, &t.from_branch, Permission::ApprovalsDecide, now)
            .ok_or_else(|| AppError::Domain("roleNotAllowed".into()))?;

        let mut tx = self.pool.begin().await?;

        let req_id_opt: Option<Uuid> = sqlx::query_scalar(
            "SELECT approval_request_id FROM cash_transfers WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

        let req_id_uuid =
            req_id_opt.ok_or_else(|| AppError::Domain("no pending approval request".into()))?;
        let req_id = RequestId::new(req_id_uuid.to_string());

        self.approvals
            .decide(&mut tx, &req_id, &policy_actor, outcome, reason, "")
            .await
            .map_err(|e| match e {
                xc_approval::application::approval_service::ApprovalError::Database(m)
                | xc_approval::application::approval_service::ApprovalError::Internal(m) => {
                    AppError::Internal(m)
                }
                other => AppError::Domain(other.to_string()),
            })?;

        tx.commit().await?;

        let updated = self.repo.find_by_id(id).await?.unwrap();
        Ok(updated)
    }

    /// Dispatches a transfer in the source branch's `BranchTx`: `dispatch_in` under that lock. The
    /// transfer is re-read and its state re-checked under the lock, and the state change is
    /// conditional on APPROVED, so a second dispatch of the same transfer is refused and posts nothing.
    pub async fn dispatch(
        &self,
        who: &Actor,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let t = self.load(id).await?;
        let parts = self.parts();
        let who = who.clone();
        let id_owned = id.to_owned();
        self.branch_tx
            .run(&t.from_branch, move |tx| {
                Box::pin(async move { Self::dispatch_on(&parts, tx, &who, &id_owned, now).await })
            })
            .await
    }

    /// Receives a transfer in the `BranchTx` of both its branches (L-5, §8.1): the receipt moves the
    /// sender's IN_TRANSIT as well as the destination, so both versions move. `receive_in` runs at the
    /// destination (the move's own branch for a single-branch kind, where the pair is one branch).
    pub async fn receive(
        &self,
        who: &Actor,
        id: &str,
        counted: Vec<NewTransferLine>,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let t = self.load(id).await?;
        let branch = destination(&t);
        let parts = self.parts();
        let who = who.clone();
        let id_owned = id.to_owned();
        let at = branch.clone();
        self.branch_tx
            .run_pair(&t.from_branch, &branch, move |tx| {
                Box::pin(async move {
                    Self::receive_on(&parts, tx, &at, &who, &id_owned, counted, now).await
                })
            })
            .await
    }

    /// Reconciles a transfer in RECEIVED status in the `BranchTx` of both its branches (as `receive`,
    /// L-5): the remainder goes to the sender's VARIANCE, so the sender's version moves too.
    pub async fn reconcile(
        &self,
        who: &Actor,
        id: &str,
        note: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let t = self.load(id).await?;
        let parts = self.parts();
        let who = who.clone();
        let id_owned = id.to_owned();
        let note = note.to_owned();
        self.branch_tx
            .run_pair(&t.from_branch, &destination(&t), move |tx| {
                Box::pin(async move {
                    Self::reconcile_on(&parts, tx, &who, &id_owned, &note, now).await
                })
            })
            .await
    }

    async fn load(&self, id: &str) -> AppResult<CashTransfer> {
        self.repo.find_by_id(id).await?.ok_or_else(|| not_found(id))
    }

    /// What dispatch, receive and reconcile need inside a `BranchTx` closure, which must own it.
    fn parts(&self) -> FlowParts {
        FlowParts {
            repo: self.repo.clone(),
            postings: self.postings.clone(),
            kinds: self.kinds.clone(),
        }
    }
}

/// The owned parts of a `TransferFlow` its locked steps run on.
#[derive(Clone)]
struct FlowParts {
    repo: Arc<dyn TransferRepository>,
    postings: Arc<dyn CashPostings>,
    kinds: TransferKinds,
}

fn not_found(id: &str) -> AppError {
    AppError::NotFound {
        entity: "CashTransfer",
        id: id.to_string(),
    }
}

/// The branch whose cash a receipt moves into: the destination of a BRANCH transfer, else the
/// move's own branch. Receive and reconcile run in its `BranchTx`, and exceptions are written on it.
fn destination(t: &CashTransfer) -> BranchId {
    t.to_branch.clone().unwrap_or_else(|| t.from_branch.clone())
}

/// The transfer re-read inside the step's transaction, refused unless `step` is legal from its
/// state now (a concurrent step may have moved it since the caller's first read).
async fn locked(
    repo: &dyn TransferRepository,
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    step: TransferStep,
) -> AppResult<CashTransfer> {
    let t = repo
        .find_by_id_in(tx, id)
        .await?
        .ok_or_else(|| not_found(id))?;
    t.status
        .next(step)
        .map_err(|e| AppError::Domain(e.to_string()))?;
    Ok(t)
}

/// Builds an in-memory copy of `t` with each line's `qty_received` set from `counted` (and 0 for
/// unmentioned lines), matching `mark_received` (D:§4.5).
fn with_counted_lines(t: &CashTransfer, counted: &[NewTransferLine]) -> CashTransfer {
    let mut updated = t.clone();
    updated.status = TransferStatus::Received;
    for line in &mut updated.lines {
        let qty = counted
            .iter()
            .rfind(|c| c.currency == line.currency && c.denomination_id == line.denomination_id)
            .map(|c| c.qty)
            .unwrap_or(0);
        line.qty_received = Some(qty);
    }
    updated
}

/// Computes the cash exception row if the counted total differs from the sent one (CA-7d #1425,
/// D:§4.5). Runs before `mark_received` so an overflowing line or currency sum refuses before any write.
fn compute_exception(
    id: &str,
    t: &CashTransfer,
    now: DateTime<Utc>,
) -> AppResult<Option<CashException>> {
    let mut sent_totals: HashMap<CurrencyCode, i64> = HashMap::new();
    let mut recv_totals: HashMap<CurrencyCode, i64> = HashMap::new();
    for line in &t.lines {
        add_line_minor(
            &mut sent_totals,
            line.currency,
            line.face_value,
            line.qty_sent,
        )
        .map_err(from_transfer_error)?;
        add_line_minor(
            &mut recv_totals,
            line.currency,
            line.face_value,
            line.qty_received.unwrap_or(0),
        )
        .map_err(from_transfer_error)?;
    }

    let mut delta_pairs = Vec::new();
    for (cur, sent_minor) in sent_totals {
        let recv_minor = recv_totals.get(&cur).copied().unwrap_or(0);
        if recv_minor != sent_minor {
            delta_pairs.push((
                cur,
                MoneyDelta::from_minor(i128::from(recv_minor) - i128::from(sent_minor)),
            ));
        }
    }

    if delta_pairs.is_empty() {
        return Ok(None);
    }

    let delta = StockDelta::of(&delta_pairs);
    let (cash_variance, safe_variance) = if t.to_shift.is_some() {
        (Some(delta), None)
    } else {
        (None, Some(delta))
    };

    Ok(Some(CashException {
        id: format!("EX-{}", Uuid::new_v4()),
        request_id: id.to_string(),
        status: ExceptionStatus::Open,
        target_difference: None,
        cash_variance,
        safe_variance,
        note: format!("TRANSFER {id}"),
        at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
        reviewed_by: None,
        reviewed_at: None,
    }))
}

/// Writes the pre-computed cash exception row if present (CA-7d #1425).
async fn write_exceptions(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    exc: Option<CashException>,
) -> AppResult<()> {
    if let Some(exc) = exc {
        append_cash_exception_in(tx, branch, &exc).await?;
    }
    Ok(())
}

/// The CA-7c forms (design §4.9): all their work, reads included, on the caller's transaction. Each assumes the
/// caller holds the branch lock CA-8 declares (dispatch: the source; receive and reconcile: the destination) and
/// takes none itself; it opens no transaction. On `Err` the caller rolls everything back.
impl TransferFlow {
    pub async fn create_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        cmd: NewTransfer,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let ends = self
            .kinds
            .get(cmd.kind)
            .ok_or_else(|| AppError::Validation(format!("unknown kind: {:?}", cmd.kind)))?;

        ends.check_create(who, &cmd)
            .map_err(|e| AppError::Domain(e.to_string()))?;

        // An idempotent replay answers the transfer already made with this key.
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM cash_transfers WHERE from_branch_id = $1 AND idempotency_key = $2",
        )
        .bind(cmd.from_branch.as_str())
        .bind(&cmd.idempotency_key)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some(id) = existing
            && let Some(t) = self.repo.find_by_id_in(tx, &id.to_string()).await?
        {
            return Ok(t);
        }

        let mut seen_denominations: HashSet<i64> = HashSet::with_capacity(cmd.lines.len());
        let mut totals: HashMap<CurrencyCode, i64> = HashMap::new();
        for line in &cmd.lines {
            if !seen_denominations.insert(line.denomination_id) {
                return Err(AppError::Validation(format!(
                    "duplicate denomination_id in transfer lines: {}",
                    line.denomination_id
                )));
            }

            let row: Option<(Decimal, String)> =
                sqlx::query_as("SELECT face_value, currency_code FROM denominations WHERE id = $1")
                    .bind(line.denomination_id)
                    .fetch_optional(&mut **tx)
                    .await?;

            let (face_dec, denom_currency) = match row {
                Some(r) => r,
                None => {
                    return Err(AppError::NotFound {
                        entity: "denomination",
                        id: line.denomination_id.to_string(),
                    });
                }
            };

            let denom_currency = denom_currency.trim();
            let line_currency = line.currency.as_str().trim();
            if !denom_currency.eq_ignore_ascii_case(line_currency) {
                return Err(AppError::Validation(format!(
                    "line currency '{line_currency}' does not match denomination currency '{denom_currency}'"
                )));
            }

            let face_value = Money::parse(&face_dec.to_string()).map_err(|e| {
                AppError::Internal(format!(
                    "invalid face_value {face_dec} for denomination {}: {e}",
                    line.denomination_id
                ))
            })?;
            add_line_minor(&mut totals, line.currency, face_value, line.qty)
                .map_err(from_transfer_error)?;
        }

        self.repo.insert(tx, &cmd, who, now).await
    }

    pub async fn submit_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let t = self
            .repo
            .find_by_id_in(tx, id)
            .await?
            .ok_or_else(|| not_found(id))?;

        if t.status != TransferStatus::Draft {
            return Err(AppError::Domain(
                TransferError::IllegalTransition {
                    from: t.status,
                    step: TransferStep::Approve,
                }
                .to_string(),
            ));
        }

        if t.created_by != who.user_id {
            return Err(AppError::Domain(
                "only the creator may submit a transfer".into(),
            ));
        }

        let policy = self
            .repo
            .find_latest_policy(tx, &who.organization_id)
            .await?;
        let within_limit = policy
            .as_ref()
            .map(|p| p.is_within_limit(&t.lines))
            .transpose()
            .map_err(from_transfer_error)?
            .unwrap_or(false);

        let payload = serde_json::json!({
            "transfer_id": id,
            "from_branch": t.from_branch.as_str(),
            "within_limit": within_limit,
        });

        let policy_actor = policy_actor(who, &t.from_branch, Permission::CashManage, now)
            .ok_or_else(|| AppError::Domain("roleNotAllowed".into()))?;
        let key = format!("submit_{}_{}", id, Uuid::new_v4());

        let req = self
            .approvals
            .request(tx, "CASH_TRANSFER", id, &payload, &policy_actor, &key)
            .await
            .map_err(|e| match e {
                xc_approval::application::approval_service::ApprovalError::Database(m)
                | xc_approval::application::approval_service::ApprovalError::Internal(m) => {
                    AppError::Internal(m)
                }
                other => AppError::Domain(other.to_string()),
            })?;

        self.repo
            .set_approval_request_id(tx, id, &req.id, now)
            .await?;

        self.repo
            .find_by_id_in(tx, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    pub async fn dispatch_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        Self::dispatch_on(&self.parts(), tx, who, id, now).await
    }

    /// `branch` is the caller's (the session) branch. A transfer whose destination is another branch is refused
    /// with `AppError::Forbidden` before anything is read under lock or written (ruling on #1290).
    pub async fn receive_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        who: &Actor,
        id: &str,
        counted: Vec<NewTransferLine>,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        Self::receive_on(&self.parts(), tx, branch, who, id, counted, now).await
    }

    pub async fn reconcile_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        id: &str,
        note: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        Self::reconcile_on(&self.parts(), tx, who, id, note, now).await
    }

    /// Checks stock and moves from the source to InTransit. The state is checked on `tx`, and the state change is
    /// conditional on APPROVED.
    async fn dispatch_on(
        parts: &FlowParts,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let t = locked(parts.repo.as_ref(), tx, id, TransferStep::Dispatch).await?;
        require(who, Permission::CashManage, &t.from_branch, now)?;
        let ends = parts
            .kinds
            .get(t.kind)
            .ok_or_else(|| AppError::Validation(format!("unknown kind: {:?}", t.kind)))?;
        let reqs = ends.on_dispatch(&t).map_err(from_transfer_error)?;

        // A move out of a drawer or the safe cannot exceed its expected balance.
        for r in &reqs {
            match &r.from {
                CashPlace::Drawer(_) | CashPlace::Safe(_) => {
                    let balances = parts.postings.expected_in(tx, &r.from).await?;
                    let balance_minor = balances.get(r.currency).try_minor().map_err(|e| {
                        AppError::Internal(format!("expected balance overflows i64: {e}"))
                    })?;
                    if Decimal::new(balance_minor, 2) < r.quantity.value() {
                        return Err(AppError::Domain(
                            TransferError::StockInsufficient.to_string(),
                        ));
                    }
                }
                CashPlace::InTransit(_)
                | CashPlace::Customer
                | CashPlace::Bank
                | CashPlace::Variance(_) => {}
            }
        }

        parts.repo.mark_dispatched(tx, id, who, now).await?;
        parts.postings.post_in(tx, &reqs, None).await?;
        parts
            .repo
            .find_by_id_in(tx, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// Updates the counted lines, moves from InTransit to the destination and writes the exceptions. The state is
    /// checked on `tx`, and the change is conditional on DISPATCHED.
    async fn receive_on(
        parts: &FlowParts,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        who: &Actor,
        id: &str,
        counted: Vec<NewTransferLine>,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let t = parts
            .repo
            .find_by_id_in(tx, id)
            .await?
            .ok_or_else(|| not_found(id))?;
        if destination(&t) != *branch {
            return Err(AppError::Forbidden);
        }
        t.status
            .next(TransferStep::Receive)
            .map_err(|e| AppError::Domain(e.to_string()))?;
        let ends = parts
            .kinds
            .get(t.kind)
            .ok_or_else(|| AppError::Validation(format!("unknown kind: {:?}", t.kind)))?;
        ends.check_receive(who, &t)
            .map_err(|e| AppError::Domain(e.to_string()))?;

        // Person rule (ruling Q1): cash entering a drawer is received by that shift's own teller, by user id.
        if let Some(ref shift_id) = t.to_shift {
            let shift_teller: String = sqlx::query_scalar(
                "SELECT teller_id::text FROM shifts WHERE id = $1 AND status = 'OPEN'",
            )
            .bind(shift_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| AppError::Domain(ShiftError::NoOpenShift.to_string()))?;
            if shift_teller != who.user_id {
                return Err(AppError::Domain(
                    "only the shift's teller may receive into drawer".into(),
                ));
            }
        }

        let counted_t = with_counted_lines(&t, &counted);
        let reqs = ends.on_receive(&counted_t).map_err(from_transfer_error)?;
        let exc = compute_exception(id, &counted_t, now)?;

        parts.repo.mark_received(tx, id, who, &counted, now).await?;
        parts.postings.post_in(tx, &reqs, None).await?;
        write_exceptions(tx, branch, exc).await?;
        parts
            .repo
            .find_by_id_in(tx, id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// Reconciles a RECEIVED transfer. The state is checked on `tx`, and the change is conditional on RECEIVED.
    async fn reconcile_on(
        parts: &FlowParts,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        id: &str,
        note: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CashTransfer> {
        let org_uuid: Uuid =
            sqlx::query_scalar("SELECT organization_id FROM cash_transfers WHERE id = $1::uuid")
                .bind(id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or_else(|| not_found(id))?;
        let organization_id = org_uuid.to_string();
        if organization_id != who.organization_id {
            return Err(not_found(id));
        }
        locked(parts.repo.as_ref(), tx, id, TransferStep::Reconcile).await?;

        let (creator_uuid, receiver_uuid): (Uuid, Option<Uuid>) = sqlx::query_as(
            "SELECT created_by, received_by FROM cash_transfers WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
        if creator_uuid.to_string() == who.user_id {
            return Err(AppError::Domain("creator cannot reconcile transfer".into()));
        }
        if receiver_uuid.map(|u| u.to_string()) == Some(who.user_id.clone()) {
            return Err(AppError::Domain(
                "receiver cannot reconcile transfer".into(),
            ));
        }

        let target = Target::new(&organization_id, None, None);
        Authorizer::check(Some(who), Permission::FinanceReconcile, &target, now)
            .map_err(|e| AppError::Domain(e.code().to_string()))?;

        parts.repo.mark_reconciled(tx, id, who, note, now).await?;
        let t = parts
            .repo
            .find_by_id_in(tx, id)
            .await?
            .ok_or_else(|| not_found(id))?;
        // What is still in transit goes to the sender's VARIANCE (§4.4 row 6): a shortfall out of
        // IN_TRANSIT, an overage back into it.
        let mut left: Vec<(CurrencyCode, Decimal)> = Vec::new();
        for line in &t.lines {
            let face_minor = line.face_value.try_minor().map_err(|e| {
                AppError::Internal(format!("reconcile face_value overflows i64: {e}"))
            })?;
            let qty = i64::from(line.qty_sent) - i64::from(line.qty_received.unwrap_or(0));
            let line_diff_minor = face_minor
                .checked_mul(qty)
                .ok_or_else(|| AppError::Internal("reconcile line total overflows i64".into()))?;
            let amount = Decimal::new(line_diff_minor, 2);
            match left.iter_mut().find(|(c, _)| *c == line.currency) {
                Some((_, a)) => {
                    *a = a.checked_add(amount).ok_or_else(|| {
                        AppError::Internal("reconcile currency total overflows".into())
                    })?;
                }
                None => left.push((line.currency, amount)),
            }
        }
        let gone: Vec<(CurrencyCode, Decimal)> = left.into_iter().map(|(c, a)| (c, -a)).collect();
        let reqs = differences(
            &CashPlace::InTransit(t.id.clone()),
            &CashPlace::Variance(t.from_branch.clone()),
            &gone,
            &SourceRef {
                source_type: SourceType::Transfer,
                source_id: t.id.clone(),
                source_key: "reconcile".to_string(),
            },
        );
        parts.postings.post_in(tx, &reqs, None).await?;
        parts
            .repo
            .find_by_id_in(tx, id)
            .await?
            .ok_or_else(|| not_found(id))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::str::FromStr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::{TimeZone, Utc};
    use sqlx::migrate::Migrator;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use sqlx::{AssertSqlSafe, PgPool};
    use xc_auth::domain::access::{Role, ScopedRole};

    use super::*;
    use crate::domain::transfer::cash_transfer::TransferKind;

    const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
    const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn unique(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after 1970")
            .as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}_{}_{nanos}_{n}", std::process::id())
    }

    fn migrations_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
    }

    async fn test_pool() -> PgPool {
        let url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| panic!("TEST_DATABASE_URL is not set"));
        let options = PgConnectOptions::from_str(&url).expect("postgres options");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("tf_reconcile_org");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("admin connect");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect schema pool");
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("migrator")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    #[tokio::test]
    async fn another_organisations_finance_gets_not_found_on_non_received_transfer() {
        let pool = test_pool().await;
        let now = Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        let from_branch = BranchId::new("XC-B01");
        let to_branch = BranchId::new("XC-B09");
        for b in [&from_branch, &to_branch] {
            sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
                .bind(b.as_str())
                .execute(&pool)
                .await
                .expect("branch of org A");
        }
        let creator_id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG_A)
        .bind(unique("orga-bm"))
        .fetch_one(&pool)
        .await
        .expect("org A creator");
        let creator = Actor::new(
            &creator_id,
            ORG_A,
            vec![ScopedRole::new(
                Role::BranchManager,
                Some(from_branch.clone()),
                now,
                None,
            )],
            None,
        );
        let usd50: i64 = sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 50",
        )
        .fetch_one(&pool)
        .await
        .expect("USD 50 denomination");

        let flow = TransferFlow::pg(pool.clone());
        let draft = flow
            .create(
                &creator,
                NewTransfer {
                    kind: TransferKind::Branch,
                    from_branch: from_branch.clone(),
                    to_branch: Some(to_branch.clone()),
                    from_shift: None,
                    to_shift: None,
                    lines: vec![NewTransferLine {
                        currency: CurrencyCode::parse("USD").unwrap(),
                        denomination_id: usd50,
                        qty: 2,
                    }],
                    note: Some("non-received transfer".into()),
                    idempotency_key: unique("k"),
                },
                now,
            )
            .await
            .expect("create DRAFT transfer in org A");

        sqlx::query(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ($1::uuid, 'ORGB', 'Asia/Bangkok', now())",
        )
        .bind(ORG_B)
        .execute(&pool)
        .await
        .expect("org B");
        let org_b_branch = BranchId::new("ORGB-B01");
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) VALUES ($1, 'B01', $2::uuid)",
        )
        .bind(org_b_branch.as_str())
        .bind(ORG_B)
        .execute(&pool)
        .await
        .expect("org B branch");
        let org_b_user_id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG_B)
        .bind(unique("orgb-fin"))
        .fetch_one(&pool)
        .await
        .expect("org B user");
        let org_b_finance = Actor::new(
            &org_b_user_id,
            ORG_B,
            vec![ScopedRole::new(
                Role::Finance,
                Some(org_b_branch),
                now,
                None,
            )],
            None,
        );

        // 1. Transfer of org A in DRAFT status -> NotFound (not IllegalTransition)
        let mut tx = pool.begin().await.expect("begin");
        let err_draft = flow
            .reconcile_in(&mut tx, &org_b_finance, &draft.id, "probe", now)
            .await
            .expect_err("org B finance must be refused on org A's DRAFT transfer");
        match err_draft {
            AppError::NotFound { entity, id } => {
                assert_eq!(entity, "CashTransfer");
                assert_eq!(id, draft.id);
            }
            other => panic!("expected AppError::NotFound on DRAFT transfer, got {other:?}"),
        }
        tx.rollback().await.expect("rollback");

        // 2. Transfer of org A in RECONCILED status -> NotFound (not IllegalTransition)
        sqlx::query("UPDATE cash_transfers SET status = 'RECONCILED' WHERE id = $1::uuid")
            .bind(&draft.id)
            .execute(&pool)
            .await
            .expect("set transfer to RECONCILED");
        let mut tx = pool.begin().await.expect("begin");
        let err_reconciled = flow
            .reconcile_in(&mut tx, &org_b_finance, &draft.id, "probe", now)
            .await
            .expect_err("org B finance must be refused on org A's RECONCILED transfer");
        match err_reconciled {
            AppError::NotFound { entity, id } => {
                assert_eq!(entity, "CashTransfer");
                assert_eq!(id, draft.id);
            }
            other => panic!("expected AppError::NotFound on RECONCILED transfer, got {other:?}"),
        }
        tx.rollback().await.expect("rollback");

        // 3. Unknown transfer id -> same NotFound shape
        let unknown = "00000000-0000-0000-0000-00000000dead";
        let mut tx = pool.begin().await.expect("begin");
        let err_unknown = flow
            .reconcile_in(&mut tx, &org_b_finance, unknown, "probe", now)
            .await
            .expect_err("unknown transfer must be refused");
        match err_unknown {
            AppError::NotFound { entity, id } => {
                assert_eq!(entity, "CashTransfer");
                assert_eq!(id, unknown);
            }
            other => panic!("expected AppError::NotFound on unknown transfer, got {other:?}"),
        }
        tx.rollback().await.expect("rollback");
    }
}
