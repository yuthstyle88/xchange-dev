//! `KioskDayCloser` (#4147, design `queue-day-close.md` §7.3): at branch day close, every
//! unfinished request of the branch is cancelled with `DAY_CLOSED` through the same `cancel_in`
//! as a staff cancel, so its ticket (SERVING included) ends `kioskCancelled:DAY_CLOSED` and a
//! `KIOSK_CANCELLED` line names `SYSTEM-DAY-CLOSE`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::kiosk_day_close::KioskDayClose;
use xc_kernel::queue_tickets::QueueTickets;

use crate::domain::kiosk::entities::kiosk_machine::KioskMachine;
use crate::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use crate::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use crate::domain::kiosk::value_objects::cancel_reason::CancelReason;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_role::KioskRole;
use crate::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

use super::desk::cancel_in;
use super::service::{cut_to_millis, time};

/// The actor of a day-close cancel: a manager, so the holder check of a held request passes.
pub const DAY_CLOSE_ACTOR: &str = "SYSTEM-DAY-CLOSE";

pub struct KioskDayCloser {
    repo: PgKioskRepository,
    ports: Arc<dyn KioskPorts>,
    queue: Arc<dyn QueueTickets>,
}

impl KioskDayCloser {
    pub fn new(
        repo: PgKioskRepository,
        ports: Arc<dyn KioskPorts>,
        queue: Arc<dyn QueueTickets>,
    ) -> Self {
        Self { repo, ports, queue }
    }

    async fn cancel_ids_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        ids: Vec<String>,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<String>> {
        let at = cut_to_millis(now);
        let now_kiosk = time(now);
        let actor = KioskActor::Staff {
            username: DAY_CLOSE_ACTOR.to_owned(),
            role: KioskRole::Manager,
        };
        let mut cancelled = Vec::with_capacity(ids.len());
        for id in ids {
            let stored = self.repo.find_in(tx, branch, &id).await?;
            if let Ok(Some(_)) =
                KioskMachine::cancel(stored.as_ref(), &actor, &CancelReason::DayClosed)
            {
                cancel_in(
                    tx,
                    &self.repo,
                    &*self.ports,
                    &*self.queue,
                    branch,
                    stored,
                    &actor,
                    CancelReason::DayClosed,
                    at,
                    now_kiosk,
                )
                .await?;
                cancelled.push(id);
            }
        }
        Ok(cancelled)
    }
}

impl KioskDayClose for KioskDayCloser {
    fn close_day_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<String>>> {
        Box::pin(async move {
            let ids: Vec<String> = sqlx::query_scalar(
                "SELECT id FROM kiosk_requests \
                 WHERE branch_id = $1 AND status IN ('WAITING','ACCEPTED','ID_CHECKED') \
                 ORDER BY created_at, id FOR UPDATE",
            )
            .bind(branch.as_str())
            .fetch_all(&mut **tx)
            .await?;

            self.cancel_ids_in(tx, branch, ids, now).await
        })
    }

    fn close_before_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        waiting_before: DateTime<Utc>,
        held_before: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<String>>> {
        Box::pin(async move {
            let ids: Vec<String> = sqlx::query_scalar(
                "SELECT r.id FROM kiosk_requests r \
                 LEFT JOIN queue_tickets t ON t.branch_id = r.branch_id AND t.kiosk_request_id = r.id \
                 WHERE r.branch_id = $1 AND ( \
                     (r.status = 'WAITING' AND COALESCE(t.issued_at, r.created_at) < $2) \
                     OR (r.status IN ('ACCEPTED','ID_CHECKED') AND COALESCE(t.issued_at, r.created_at) < $3) \
                 ) \
                 ORDER BY r.id FOR UPDATE OF r",
            )
            .bind(branch.as_str())
            .bind(waiting_before)
            .bind(held_before)
            .fetch_all(&mut **tx)
            .await?;

            self.cancel_ids_in(tx, branch, ids, now).await
        })
    }
}
