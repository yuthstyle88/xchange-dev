//! The desk's side of the queue: notifications, list, accept, identity check, get and publish the receipt.
//! Methods of `KioskService` (design §10 S0); the type lives in `service.rs`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::{KioskTicketOutcome, QueueTickets};

use crate::domain::kiosk::commands::kiosk_command::KioskCommand;
use crate::domain::kiosk::entities::kiosk_machine::KioskMachine;
use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use crate::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use crate::domain::kiosk::value_objects::accept_outcome::AcceptOutcome;
use crate::domain::kiosk::value_objects::audit_line::{AuditLine, KioskAudit};
use crate::domain::kiosk::value_objects::cancel_reason::{CancelReason, Cancellation};
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_error::KioskError;
use crate::domain::kiosk::value_objects::kiosk_service_error::KioskServiceError;
use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::domain::kiosk::value_objects::notifications::Notifications;
use crate::domain::kiosk::value_objects::queue_filter::QueueFilter;
use crate::domain::kiosk::value_objects::queue_page::{QueueItem, QueuePage};
use crate::domain::kiosk::value_objects::receipt_image::ReceiptImage;
use crate::domain::kiosk::value_objects::request_identity::RequestIdentity;
use crate::domain::kiosk::value_objects::work_times::WorkTimes;
use crate::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

use super::service::{KioskService, NO_COUNTER, cut_to_millis, line_for, refuse, time, username};

impl KioskService {
    /// `notifications`: how many requests wait, who is working, for anyone signed in.
    pub async fn notifications(
        &self,
        branch: &BranchId,
        _actor: &KioskActor,
    ) -> AppResult<Notifications> {
        let branch_name = self.branch_name(branch).await?;
        let (count, latest) = self.repo.waiting(branch).await?;
        let (working, working_count) = self.repo.working(branch, 10).await?;
        Ok(Notifications {
            branch_name,
            count,
            latest,
            working,
            working_count,
        })
    }

    /// `list`: the queue of the branch, filtered and paged, at most 50 items.
    pub async fn list(
        &self,
        branch: &BranchId,
        _actor: &KioskActor,
        filter: QueueFilter,
        page: u32,
        now: DateTime<Utc>,
    ) -> AppResult<QueuePage> {
        self.branch_name(branch).await?;
        let now = time(now);
        let mut rows = self.repo.queue_page(branch, filter, page).await?;
        let has_more = rows.len() == 51;
        if has_more {
            rows.pop();
        }
        let mut items = Vec::with_capacity(rows.len());
        for (request, times) in rows {
            let identity = KioskMachine::identity(Some(&request), now).map_err(refuse)?;
            items.push(QueueItem {
                request,
                identity,
                times,
            });
        }
        Ok(QueuePage {
            page,
            has_more,
            items,
        })
    }

    /// `queue_before`: the queue of the branch, filtered and keyset paged by `before`, at most `limit` items.
    pub async fn queue_before(
        &self,
        branch: &BranchId,
        filter: QueueFilter,
        before: Option<&str>,
        limit: i64,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<QueueItem>> {
        self.branch_name(branch).await?;
        let now = time(now);
        let rows = self
            .repo
            .queue_before(branch, filter, before, limit)
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for (request, times) in rows {
            let identity = KioskMachine::identity(Some(&request), now).map_err(refuse)?;
            items.push(QueueItem {
                request,
                identity,
                times,
            });
        }
        Ok(items)
    }

    /// `accept`: a desk employee takes a waiting request.
    pub async fn accept(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<AcceptOutcome> {
        self.accept_with(branch, actor, id, now, None).await
    }

    /// `accept` that also starts the request's queue ticket in the same transaction, through the
    /// queue's port (design `queue-tickets.md` §4.5): a WAITING ticket is called first. `queue`
    /// acts for the accepting teller; when the queue refuses (no counter, a paused counter, a
    /// ticket already at the counter, …) nothing is written and the request stays WAITING.
    pub async fn accept_with_queue(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        now: DateTime<Utc>,
        queue: Arc<dyn QueueTickets>,
    ) -> AppResult<AcceptOutcome> {
        self.accept_with(branch, actor, id, now, Some(queue)).await
    }

    async fn accept_with(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        now: DateTime<Utc>,
        queue: Option<Arc<dyn QueueTickets>>,
    ) -> AppResult<AcceptOutcome> {
        let user = match actor {
            KioskActor::Staff { username, .. } => username.clone(),
            KioskActor::Customer => return Err(refuse(KioskError::StaffRequired)),
        };
        self.branch_name(branch).await?;
        let at = cut_to_millis(now);
        let now_kiosk = time(now);
        let (repo, ports, b, id, actor) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            id.to_owned(),
            actor.clone(),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let stored = repo.find_in(tx, &b, &id).await?;
                    let req = match stored {
                        Some(r) => r,
                        None => return Ok(AcceptOutcome::Unavailable),
                    };

                    match req.status {
                        KioskStatus::Quoted | KioskStatus::Expired | KioskStatus::Cancelled => {
                            Ok(AcceptOutcome::Unavailable)
                        }
                        KioskStatus::Completed => {
                            let holder = req.staff.unwrap_or_default();
                            Ok(AcceptOutcome::HeldBy(holder))
                        }
                        KioskStatus::Accepted | KioskStatus::IdChecked => {
                            let current_holder = req.staff.as_deref().unwrap_or("");
                            if current_holder == user {
                                let times = repo.times_in(tx, &b, &id).await?;
                                Ok(AcceptOutcome::AlreadyMine(times))
                            } else {
                                Ok(AcceptOutcome::HeldBy(current_holder.to_owned()))
                            }
                        }
                        KioskStatus::Waiting => {
                            let moved = KioskMachine::next(
                                Some(&req),
                                &KioskCommand::Accept,
                                &actor,
                                &NO_COUNTER,
                                now_kiosk,
                            )
                            .map_err(refuse)?;
                            if let Some(queue) = &queue {
                                queue
                                    .start_for_kiosk_in(tx, &b, &id, at)
                                    .await
                                    .map_err(queue_refusal)?;
                            }
                            if let Some(event) = moved.event {
                                repo.update_in(tx, &b, &moved.request).await?;
                                repo.mark_started_in(tx, &b, &id, at).await?;
                                let line = line_for(event, &moved.request, username(&actor));
                                ports.record_in(tx, &b, &line, at).await?;
                            }
                            let times = WorkTimes {
                                started_at: Some(now_kiosk),
                                completed_at: None,
                            };
                            Ok(AcceptOutcome::Claimed(times))
                        }
                    }
                })
            })
            .await
    }

    /// `cancel`: staff cancel a waiting or held request with a reason, ending its walk-in ticket
    /// and recording `KIOSK_CANCELLED` in one transaction (`kiosk-cancel.md` §3.3).
    pub async fn cancel_with(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        reason: CancelReason,
        now: DateTime<Utc>,
        queue: Arc<dyn QueueTickets>,
    ) -> AppResult<RequestIdentity> {
        self.branch_name(branch).await?;
        let at = cut_to_millis(now);
        let now_kiosk = time(now);
        let (repo, ports, b, id, actor) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            id.to_owned(),
            actor.clone(),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let stored = repo.find_in(tx, &b, &id).await?;
                    cancel_in(
                        tx, &repo, &*ports, &*queue, &b, stored, &actor, reason, at, now_kiosk,
                    )
                    .await
                })
            })
            .await
    }

    /// `identityChecked`: the employee holding the request confirms the passport check.
    pub async fn identity_checked(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        confirmed: bool,
        now: DateTime<Utc>,
    ) -> AppResult<RequestIdentity> {
        self.desk_step(
            branch,
            actor,
            id,
            KioskCommand::IdentityChecked { confirmed },
            now,
        )
        .await
    }

    async fn desk_step(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        command: KioskCommand,
        now: DateTime<Utc>,
    ) -> AppResult<RequestIdentity> {
        self.branch_name(branch).await?;
        let at = cut_to_millis(now);
        let now = time(now);
        let (repo, ports, b, id, actor) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            id.to_owned(),
            actor.clone(),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let stored = repo.find_in(tx, &b, &id).await?;
                    let moved =
                        KioskMachine::next(stored.as_ref(), &command, &actor, &NO_COUNTER, now)
                            .map_err(refuse)?;
                    if let Some(event) = moved.event {
                        repo.update_in(tx, &b, &moved.request).await?;
                        let line = line_for(event, &moved.request, username(&actor));
                        ports.record_in(tx, &b, &line, at).await?;
                    }
                    KioskMachine::identity(Some(&moved.request), now).map_err(refuse)
                })
            })
            .await
    }

    /// `get`: the request the employee holds, to fill the transaction form from.
    pub async fn get(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
    ) -> AppResult<KioskRequest> {
        self.branch_name(branch).await?;
        let stored = self.repo.find(branch, id).await?;
        KioskMachine::check_get(stored.as_ref(), actor).map_err(refuse)?;
        stored.ok_or_else(|| refuse(KioskError::AcceptFirst))
    }

    /// `publishReceipt`: the employee holding the request sends the receipt
    /// picture to the customer's screen.
    pub async fn publish_receipt(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        image: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.branch_name(branch).await?;
        let now = time(now);
        let image = ReceiptImage::parse(image);
        let (repo, b, id, actor) = (
            self.repo.clone(),
            branch.clone(),
            id.to_owned(),
            actor.clone(),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let stored = repo.find_in(tx, &b, &id).await?;
                    KioskMachine::check_publish(stored.as_ref(), &actor).map_err(refuse)?;
                    let image =
                        image.ok_or_else(|| refuse(KioskServiceError::InvalidReceiptImage))?;
                    repo.put_receipt_in(tx, &b, &id, &image, now).await
                })
            })
            .await
    }
}

/// The queue's refusals reach the desk as domain refusals, as the kiosk's `confirm` answers them;
/// a caller the queue refuses (`QueueOperate`) gets the queue's v3 code `outOfScope`.
fn queue_refusal(err: AppError) -> AppError {
    match err {
        AppError::Validation(code) | AppError::Conflict(code) => AppError::Domain(code),
        AppError::Forbidden => AppError::Domain("outOfScope".to_owned()),
        other => other,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn cancel_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    repo: &PgKioskRepository,
    ports: &dyn KioskPorts,
    queue: &dyn QueueTickets,
    branch: &BranchId,
    stored: Option<KioskRequest>,
    actor: &KioskActor,
    reason: CancelReason,
    at: DateTime<Utc>,
    now_kiosk: KioskTime,
) -> AppResult<RequestIdentity> {
    let moved = match KioskMachine::cancel(stored.as_ref(), actor, &reason).map_err(refuse)? {
        Some(moved) => moved,
        None => return KioskMachine::identity(stored.as_ref(), now_kiosk).map_err(refuse),
    };
    let from = stored.as_ref().expect("stored when Some").status;
    let who = username(actor).to_owned();
    let cancel = Cancellation {
        at,
        by: who.clone(),
        reason: reason.clone(),
    };
    if !repo
        .mark_cancelled_in(tx, branch, &moved.id, from, &cancel)
        .await?
    {
        return Err(refuse(KioskError::StaleVersion));
    }
    let outcome = if reason.is_no_show() {
        KioskTicketOutcome::NoShow
    } else {
        KioskTicketOutcome::Cancelled {
            reason: reason.as_str().to_owned(),
        }
    };
    queue
        .cancel_active_for_kiosk_in(tx, branch, &moved.id, &outcome, at)
        .await
        .map_err(queue_refusal)?;
    let line = AuditLine {
        actor: who,
        subject: moved.id.clone(),
        what: KioskAudit::Cancelled { from, reason },
    };
    ports.record_in(tx, branch, &line, at).await?;
    KioskMachine::identity(Some(&moved), now_kiosk).map_err(refuse)
}
