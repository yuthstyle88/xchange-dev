//! Kiosk service.
//!
//! Every method names the branch it works on and checks that the branch exists
//! before anything else it reads. Who may call what is `Policy`, asked by the
//! route. A refusal is `AppError::Domain(<the sentence>)`. Every
//! write runs inside `BranchTx::run` of that branch; `check_save_in` and
//! `complete_in` run inside the transaction of the save that calls them.

use std::fmt::Display;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use crate::domain::kiosk::value_objects::audit_line::{AuditLine, CUSTOMER_KIOSK, KioskAudit};
use crate::domain::kiosk::value_objects::counter_facts::CounterFacts;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_event_kind::KioskEventKind;
use crate::domain::kiosk::value_objects::kiosk_service_error::KioskServiceError;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

/// How many requests the queue shows.
pub const QUEUE_LIMIT: i64 = 100;

/// A counter nobody needs to look at: closed, no rate.
pub(super) const NO_COUNTER: CounterFacts = CounterFacts {
    open: false,
    line: None,
};

pub(super) fn refuse(sentence: impl Display) -> AppError {
    AppError::Domain(sentence.to_string())
}

pub(super) fn no_such_branch() -> AppError {
    refuse(KioskServiceError::NoSuchBranch)
}

/// The instant as the kiosk keeps it: whole milliseconds.
pub(super) fn time(now: DateTime<Utc>) -> KioskTime {
    KioskTime::from_epoch_millis(now.timestamp_millis())
}

pub(super) fn cut_to_millis(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(now.timestamp_millis())
        .single()
        .unwrap_or(now)
}

pub(super) fn username(actor: &KioskActor) -> &str {
    match actor {
        KioskActor::Staff { username, .. } => username,
        KioskActor::Customer => CUSTOMER_KIOSK,
    }
}

/// The audit line of a transition: who is named and what is kept with it.
pub(super) fn line_for(event: KioskEventKind, request: &KioskRequest, actor: &str) -> AuditLine {
    let (actor, what) = match event {
        KioskEventKind::Request => (
            CUSTOMER_KIOSK,
            KioskAudit::Request {
                quote: request.quote,
            },
        ),
        KioskEventKind::Accepted => (actor, KioskAudit::Accepted),
        KioskEventKind::IdChecked => (actor, KioskAudit::IdChecked),
        KioskEventKind::Completed => (
            actor,
            KioskAudit::Completed {
                document_ref: request
                    .transaction
                    .as_ref()
                    .map(|link| link.document_ref.clone())
                    .unwrap_or_default(),
            },
        ),
    };
    AuditLine {
        actor: actor.to_owned(),
        subject: request.id.clone(),
        what,
    }
}

#[derive(Clone)]
pub struct KioskService {
    pub(super) branch_tx: BranchTx,
    pub(super) repo: PgKioskRepository,
    pub(super) ports: Arc<dyn KioskPorts>,
}
impl KioskService {
    pub fn new(branch_tx: BranchTx, ports: Arc<dyn KioskPorts>) -> Self {
        let repo = PgKioskRepository::new(branch_tx.pool().clone());
        Self {
            branch_tx,
            repo,
            ports,
        }
    }

    pub(super) async fn branch_name(&self, branch: &BranchId) -> AppResult<String> {
        self.ports
            .branch_name(branch)
            .await?
            .ok_or_else(no_such_branch)
    }
}
