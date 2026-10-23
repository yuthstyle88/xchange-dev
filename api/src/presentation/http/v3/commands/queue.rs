//! QU-7 (#1266): the queue's v3 commands (design `docs/design/blueprint/queue-tickets.md` §4.4).
//!
//! One flat file, as `api/build.rs` registers only flat files under `commands/` (G6,
//! `cash-shift-transfer.md:262-270`; the shape of `denomination_groups.rs`):
//! - the walk-in issue and the desk's ticket commands (`QueueDesk` QU-3, `TicketIssuer` QU-2);
//! - pause and resume of the caller's counter (`CounterService` QU-4);
//! - opening and closing the queue (`StatusService` QU-5).
//!
//! Each runs on the seam's transaction, which already holds the branch's lock, through the
//! services' `*_in` forms, and inherits its own `(Rpc, "<name>")` row in
//! `xc-auth/src/domain/policy/areas/queue.rs`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor as User;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_queue::application::counter::counter_service::CounterService;
use xc_queue::application::desk::queue_desk::QueueDesk;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::application::status::status_service::StatusService;
use xc_queue::domain::counter::counter::Counter;
use xc_queue::domain::repository::QueueRepository;
use xc_queue::domain::status::branch_service_status::BranchServiceStatus;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::QueueTicket;
use xc_queue::infrastructure::repository::PgQueueRepository;

use xc_booking::infrastructure::lifecycle_factory::pg_booking_lifecycle;
use xc_notify::application::notification_service::NotificationService;
use xc_notify::application::template_registry::TemplateRegistry;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input::{self, TextBound};

// ------------------------------------------------------------ shared

/// The queue's services on one pool.
#[derive(Clone)]
pub struct QueueServices {
    pub desk: QueueDesk,
    pub counters: CounterService,
    pub status: StatusService,
    pub issuer: TicketIssuer,
}

impl QueueServices {
    pub fn new(pool: PgPool) -> Self {
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        let repo: Arc<dyn QueueRepository> = Arc::new(PgQueueRepository::new(pool.clone()));
        let status = StatusService::new(branch_tx.clone(), repo.clone());
        let notifications = Arc::new(NotificationService::new(
            pool.clone(),
            Arc::new(TemplateRegistry::generated()),
        ));
        let booking_events: Arc<dyn xc_kernel::booking_queue_events::BookingQueueEvents> =
            pg_booking_lifecycle(pool);
        Self {
            desk: QueueDesk::new(branch_tx.clone(), repo.clone(), status.clone())
                .with_booking_events(booking_events.clone())
                .with_notifications(notifications),
            counters: CounterService::new(branch_tx.clone(), repo.clone(), status.clone()),
            issuer: TicketIssuer::new(branch_tx, repo).with_booking_events(booking_events),
            status,
        }
    }
}

/// The Blueprint actor the queue's services authorise (`Authorizer::check`): the seam's own actor
/// (#1030), once the branch is known.
pub async fn user_in(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    who: &User,
) -> Result<User, V3Error> {
    let organization: Option<String> =
        sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| server_error())?;
    organization.ok_or_else(out_of_scope)?;
    Ok(who.clone())
}

/// A queue refusal as v3 answers it: the queue's code (`QueueError::code`), with the status its
/// kind of refusal takes.
pub fn refusal(err: AppError) -> V3Error {
    let (code, status): (&'static str, StatusCode) = match &err {
        AppError::Forbidden => return out_of_scope(),
        AppError::NotFound { .. } => ("notFound", StatusCode::NOT_FOUND),
        AppError::Conflict(c) | AppError::Validation(c) => match c.as_str() {
            "staleVersion" => ("staleVersion", StatusCode::CONFLICT),
            "idempotencyConflict" => ("idempotencyConflict", StatusCode::CONFLICT),
            "notFound" => ("notFound", StatusCode::NOT_FOUND),
            "queueClosed" => ("queueClosed", StatusCode::CONFLICT),
            "counterPaused" => ("counterPaused", StatusCode::CONFLICT),
            "alreadyServing" => ("alreadyServing", StatusCode::CONFLICT),
            "noOpenShift" => ("noOpenShift", StatusCode::CONFLICT),
            "notYourTicket" => ("notYourTicket", StatusCode::CONFLICT),
            "nothingWaiting" => ("nothingWaiting", StatusCode::CONFLICT),
            _ => return unexpected(err),
        },
        _ => return unexpected(err),
    };
    V3Error::new(code, None, code, status)
}

fn unexpected(err: AppError) -> V3Error {
    tracing::error!(error = %err, "queue command failed");
    server_error()
}

/// A ticket as a queue command answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue.ts")]
pub struct QueueTicketDto {
    pub id: String,
    pub display_number: String,
    /// `WALK_IN` or `BOOKING`.
    pub service_type: String,
    pub sequence: i32,
    /// `WAITING`, `CALLED`, `SERVING`, `COMPLETED`, `NO_SHOW` or `CANCELLED`.
    pub status: String,
    /// The teller whose counter holds it (`users.id`).
    pub teller_id: Option<String>,
    pub ended_reason: Option<String>,
}

impl From<QueueTicket> for QueueTicketDto {
    fn from(t: QueueTicket) -> Self {
        Self {
            id: t.id,
            display_number: t.display_number,
            service_type: t.service_type.as_str().to_owned(),
            sequence: t.sequence,
            status: t.status.as_str().to_owned(),
            teller_id: t.counter_teller_id,
            ended_reason: t.ended_reason,
        }
    }
}

/// The caller's counter after pause or resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue.ts")]
pub struct QueueCounterDto {
    pub teller_id: String,
    /// `ACTIVE` or `PAUSED`.
    pub status: String,
}

impl From<Counter> for QueueCounterDto {
    fn from(c: Counter) -> Self {
        Self {
            teller_id: c.teller_id,
            status: c.status.as_str().to_owned(),
        }
    }
}

/// The queue's state after open or close.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue.ts")]
pub struct QueueStateDto {
    pub queue_open: bool,
    pub active_counters: i32,
}

impl From<BranchServiceStatus> for QueueStateDto {
    fn from(s: BranchServiceStatus) -> Self {
        Self {
            queue_open: s.queue_open,
            active_counters: s.active_counters,
        }
    }
}

/// A command with no input of its own (the seam takes `branch`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue.ts")]
pub struct QueueNoInput {}

/// A command on one ticket: `id` is the route's `{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue.ts")]
pub struct QueueTicketInput {
    pub id: String,
}

/// `cancel`: the ticket and why it ends (`queue_tickets.ended_reason`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue.ts")]
pub struct QueueCancelInput {
    pub id: String,
    pub reason: String,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    QueueTicketDto::export_all(&cfg)?;
    QueueCounterDto::export_all(&cfg)?;
    QueueStateDto::export_all(&cfg)?;
    QueueNoInput::export_all(&cfg)?;
    QueueTicketInput::export_all(&cfg)?;
    QueueCancelInput::export_all(&cfg)?;
    Ok(())
}

// ------------------------------------------------------------ tickets

/// `POST /api/queue/tickets`: the desk issues a walk-in ticket for a customer without the tablet
/// (`IssueSource::Desk`). The command's Idempotency-Key is the ticket's key, so a replay issues
/// nothing new.
pub struct IssueWalkIn(pub QueueServices);

impl CommandAction for IssueWalkIn {
    const NAME: &'static str = "ticketIssueWalkIn";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ticketIssueWalkIn");
    type Input = QueueNoInput;
    type Output = QueueTicketDto;
    const ROUTE: &'static str = "/api/queue/tickets";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        _who: &User,
        _input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        self.0
            .issuer
            .issue_in(tx, branch, &IssueSource::Desk, key.as_str(), Utc::now())
            .await
            .map(QueueTicketDto::from)
            .map_err(refusal)
    }
}

/// `POST /api/queue/call-next`: the first WAITING ticket in call order.
pub struct CallNext(pub QueueServices);

impl CommandAction for CallNext {
    const NAME: &'static str = "ticketCallNext";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ticketCallNext");
    type Input = QueueNoInput;
    type Output = QueueTicketDto;
    const ROUTE: &'static str = "/api/queue/call-next";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &User,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let user = user_in(tx, branch, who).await?;
        self.0
            .desk
            .call_next_in(tx, &user, branch, Utc::now())
            .await
            .map(QueueTicketDto::from)
            .map_err(refusal)
    }
}

fn ticket_id(raw: &str) -> Result<String, V3Error> {
    Uuid::parse_str(raw).map(|id| id.to_string()).map_err(|_| {
        V3Error::new(
            "validationFailed",
            Some("id"),
            "invalid ticket id",
            StatusCode::BAD_REQUEST,
        )
    })
}

/// A desk command on one ticket, `POST /api/queue/tickets/{id}/<verb>`.
macro_rules! ticket_command {
    ($(#[$doc:meta])* $ty:ident, $name:literal, $route:literal, $method:ident) => {
        $(#[$doc])*
        pub struct $ty(pub QueueServices);

        impl CommandAction for $ty {
            const NAME: &'static str = $name;
            const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, $name);
            type Input = QueueTicketInput;
            type Output = QueueTicketDto;
            const ROUTE: &'static str = $route;
            const METHOD: Method = Method::POST;

            async fn run(
                &self,
                tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
                branch: &BranchId,
                who: &User,
                input: Self::Input,
                _key: IdempotencyKey,
            ) -> Result<Self::Output, V3Error> {
                let id = ticket_id(&input.id)?;
                let user = user_in(tx, branch, who).await?;
                self.0
                    .desk
                    .$method(tx, &user, branch, &id, Utc::now())
                    .await
                    .map(QueueTicketDto::from)
                    .map_err(refusal)
            }
        }
    };
}

ticket_command!(
    /// A chosen WAITING ticket → CALLED.
    Call, "ticketCall", "/api/queue/tickets/{id}/call", call_in
);
ticket_command!(
    /// CALLED → CALLED, with a RECALL event.
    Recall, "ticketRecall", "/api/queue/tickets/{id}/recall", recall_in
);
ticket_command!(
    /// CALLED → NO_SHOW, final.
    Skip, "ticketSkip", "/api/queue/tickets/{id}/skip", skip_in
);
ticket_command!(
    /// CALLED → SERVING.
    Start, "ticketStart", "/api/queue/tickets/{id}/start", start_in
);
ticket_command!(
    /// SERVING → COMPLETED.
    Finish, "ticketFinish", "/api/queue/tickets/{id}/finish", finish_in
);

/// `POST /api/queue/tickets/{id}/cancel`: WAITING or CALLED → CANCELLED, with `reason`.
pub struct Cancel(pub QueueServices);

impl CommandAction for Cancel {
    const NAME: &'static str = "ticketCancel";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ticketCancel");
    type Input = QueueCancelInput;
    type Output = QueueTicketDto;
    const ROUTE: &'static str = "/api/queue/tickets/{id}/cancel";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &User,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let id = ticket_id(&input.id)?;
        let reason = input::text("reason", &input.reason, TextBound { min: 1, max: 200 })?;
        let user = user_in(tx, branch, who).await?;
        self.0
            .desk
            .cancel_in(tx, &user, branch, &id, reason, Utc::now())
            .await
            .map(QueueTicketDto::from)
            .map_err(refusal)
    }
}

// ------------------------------------------------------------ the counter

/// `POST /api/queue/counter/pause`: the caller's counter → PAUSED; it stops calling.
pub struct Pause(pub QueueServices);

impl CommandAction for Pause {
    const NAME: &'static str = "counterPause";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "counterPause");
    type Input = QueueNoInput;
    type Output = QueueCounterDto;
    const ROUTE: &'static str = "/api/queue/counter/pause";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &User,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let user = user_in(tx, branch, who).await?;
        self.0
            .counters
            .pause_in(tx, &user, branch, Utc::now())
            .await
            .map(QueueCounterDto::from)
            .map_err(refusal)
    }
}

/// `POST /api/queue/counter/resume`: the caller's counter → ACTIVE again.
pub struct Resume(pub QueueServices);

impl CommandAction for Resume {
    const NAME: &'static str = "counterResume";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "counterResume");
    type Input = QueueNoInput;
    type Output = QueueCounterDto;
    const ROUTE: &'static str = "/api/queue/counter/resume";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &User,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let user = user_in(tx, branch, who).await?;
        self.0
            .counters
            .resume_in(tx, &user, branch, Utc::now())
            .await
            .map(QueueCounterDto::from)
            .map_err(refusal)
    }
}

// ------------------------------------------------------------ the day

/// `POST /api/queue/open`: new tickets may be issued.
pub struct Open(pub QueueServices);

impl CommandAction for Open {
    const NAME: &'static str = "queueOpen";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "queueOpen");
    type Input = QueueNoInput;
    type Output = QueueStateDto;
    const ROUTE: &'static str = "/api/queue/open";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &User,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let user = user_in(tx, branch, who).await?;
        self.0
            .status
            .open_in(tx, &user, branch, Utc::now())
            .await
            .map(QueueStateDto::from)
            .map_err(refusal)
    }
}

/// `POST /api/queue/close`: no new ticket; the tickets already issued are still served (Q8).
pub struct Close(pub QueueServices);

impl CommandAction for Close {
    const NAME: &'static str = "queueClose";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "queueClose");
    type Input = QueueNoInput;
    type Output = QueueStateDto;
    const ROUTE: &'static str = "/api/queue/close";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &User,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let user = user_in(tx, branch, who).await?;
        self.0
            .status
            .close_in(tx, &user, branch, Utc::now())
            .await
            .map(QueueStateDto::from)
            .map_err(refusal)
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let q = QueueServices::new(state.db.clone());
    CommandEndpoint::register(cfg, Arc::new(IssueWalkIn(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(CallNext(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Call(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Recall(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Skip(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Start(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Finish(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Cancel(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Pause(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Resume(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Open(q.clone())));
    CommandEndpoint::register(cfg, Arc::new(Close(q)));
}
