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

use crate::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use crate::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

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
}

impl KioskDayClose for KioskDayCloser {
    fn close_day_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<String>>> {
        let _ = (&self.repo, &self.ports, &self.queue, tx, branch, now);
        todo!("KX #4147")
    }
}
