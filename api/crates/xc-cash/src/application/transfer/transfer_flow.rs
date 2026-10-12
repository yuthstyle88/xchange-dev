//! `TransferFlow`: orchestrates cash transfers across the 5 states (D:§4.5, §9.1 CA-5).

use std::collections::HashMap;
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
use xc_kernel::money::{CurrencyCode, MoneyDelta};
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
use xc_shift::infrastructure::custody::custody_repository::append_cash_exception_in;

use crate::application::approval::{TransferApprovalPolicy, TransferApprovalSubject, policy_actor};
use crate::domain::ledger::posting::{SourceRef, SourceType};
use crate::domain::posting::{CashPlace, CashPostings, differences};
use crate::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferError, TransferStatus, TransferStep,
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
            .map_err(|e| AppError::Domain(e.to_string()))?;

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

/// Writes a cash exception row if the counted total differs from the sent one (CA-7d #1425).
async fn write_exceptions(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    branch: &BranchId,
    t: &CashTransfer,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let mut sent_totals: HashMap<CurrencyCode, i64> = HashMap::new();
    let mut recv_totals: HashMap<CurrencyCode, i64> = HashMap::new();
    for line in &t.lines {
        let sent_minor = line.face_value.minor() * (line.qty_sent as i64);
        *sent_totals.entry(line.currency).or_insert(0) += sent_minor;
        let recv_minor = line.face_value.minor() * (line.qty_received.unwrap_or(0) as i64);
        *recv_totals.entry(line.currency).or_insert(0) += recv_minor;
    }

    let mut delta_pairs = Vec::new();
    for (cur, sent_minor) in sent_totals {
        let recv_minor = recv_totals.get(&cur).copied().unwrap_or(0);
        if recv_minor != sent_minor {
            delta_pairs.push((
                cur,
                MoneyDelta::from_minor((recv_minor - sent_minor) as i128),
            ));
        }
    }

    if !delta_pairs.is_empty() {
        let delta = StockDelta::of(&delta_pairs);
        let (cash_variance, safe_variance) = if t.to_shift.is_some() {
            (Some(delta), None)
        } else {
            (None, Some(delta))
        };

        let exc = CashException {
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
        };
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
            .map_err(|e| AppError::Domain(e.to_string()))?;

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
        let reqs = ends.on_dispatch(&t);

        // A move out of a drawer or the safe cannot exceed its expected balance.
        for r in &reqs {
            match &r.from {
                CashPlace::Drawer(_) | CashPlace::Safe(_) => {
                    let balances = parts.postings.expected_in(tx, &r.from).await?;
                    if Decimal::new(balances.get(r.currency).minor(), 2) < r.quantity.value() {
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
            let shift_teller: String =
                sqlx::query_scalar("SELECT teller_id::text FROM shifts WHERE id = $1")
                    .bind(shift_id)
                    .fetch_one(&mut **tx)
                    .await?;
            if shift_teller != who.user_id {
                return Err(AppError::Domain(
                    "only the shift's teller may receive into drawer".into(),
                ));
            }
        }

        parts.repo.mark_received(tx, id, who, &counted, now).await?;
        let updated_t = parts
            .repo
            .find_by_id_in(tx, id)
            .await?
            .ok_or_else(|| not_found(id))?;
        let reqs = ends.on_receive(&updated_t);
        parts.postings.post_in(tx, &reqs, None).await?;
        write_exceptions(tx, id, branch, &updated_t, now).await?;
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

        let target = Target::new(&who.organization_id, None, None);
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
            let qty = line.qty_sent - line.qty_received.unwrap_or(0);
            let amount = Decimal::new(line.face_value.minor(), 2) * Decimal::from(qty);
            match left.iter_mut().find(|(c, _)| *c == line.currency) {
                Some((_, a)) => *a += amount,
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
