//! The seven branch-day commands on the CM-1 seam (CA-8b #1274, design `cash-shift-transfer.md` §3.2, §4.4, §9 CA-8b).
//!
//! Wrapping CA-7b's `BranchDayService` (#1273):
//! - `dayStartService` `POST /api/branch-day/start`
//! - `dayCutoff` `POST /api/branch-day/cutoff`
//! - `dayTarget` `POST /api/branch-day/target`
//! - `clock` `POST /api/attendance`
//! - `holdCreate` `POST /api/customer-holds`
//! - `holdRelease` `POST /api/customer-holds/{id}/release`
//! - `draftBegin` `POST /api/tx-drafts`

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{DateTime, Utc};
use pos_infra::transactions::Transactor;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Side};
use xc_shift::application::branch_day::service::{
    BeginDraft, BranchDayService, ClockDirection, StartService,
};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::custody::repositories::custody_ports::CustodyPorts;
use xc_shift::domain::custody::value_objects::custody_error::CustodyError;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::bootstrap::AppState;
use crate::infrastructure::custody::custody_ports::PgCustodyPorts;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::branch_day::{
    AttendanceClockInput, AttendanceClockOutput, CustomerHoldCreateInput, CustomerHoldCreateOutput,
    CustomerHoldReleaseInput, CustomerHoldReleaseOutput, DayCutoffInput, DayCutoffOutput,
    DayStartServiceInput, DayStartServiceOutput, DayTargetInput, DayTargetOutput, DraftBeginInput,
    DraftBeginOutput,
};
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

fn map_service_error(err: pos_error::AppError) -> V3Error {
    match err {
        pos_error::AppError::Domain(ref code) if code == "roleNotAllowed" => V3Error::from(
            xc_auth::domain::policy::value_objects::policy_error::PolicyError::RoleNotAllowed,
        ),
        pos_error::AppError::Domain(ref code) if code == "outOfScope" => V3Error::new(
            "outOfScope",
            None,
            "ไม่มีสิทธิ์ทำรายการสำหรับสาขานี้",
            StatusCode::FORBIDDEN,
        ),
        pos_error::AppError::Forbidden => V3Error::new(
            "outOfScope",
            None,
            "ไม่มีสิทธิ์ทำรายการสำหรับสาขานี้",
            StatusCode::FORBIDDEN,
        ),
        pos_error::AppError::Domain(message) => {
            V3Error::new("validationFailed", None, message, StatusCode::BAD_REQUEST)
        }
        pos_error::AppError::Validation(message) => {
            V3Error::new("validationFailed", None, message, StatusCode::BAD_REQUEST)
        }
        pos_error::AppError::NotFound { entity, id } => V3Error::new(
            "notFound",
            None,
            format!("{entity} {id} not found"),
            StatusCode::NOT_FOUND,
        ),
        pos_error::AppError::Conflict(message) => {
            V3Error::new("conflict", None, message, StatusCode::CONFLICT)
        }
        other => {
            tracing::error!(error = %other, "branch day command failed");
            server_error()
        }
    }
}

// ------------------------------------------------------------ 1. dayStartService

pub struct DayStartServiceCommand {
    service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>,
    ports: Arc<dyn CustodyPorts>,
}

impl DayStartServiceCommand {
    pub fn new(
        service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>,
        ports: Arc<dyn CustodyPorts>,
    ) -> Self {
        Self { service, ports }
    }
}

impl CommandAction for DayStartServiceCommand {
    const NAME: &'static str = "dayStartService";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, Self::NAME);
    type Input = DayStartServiceInput;
    type Output = DayStartServiceOutput;
    const ROUTE: &'static str = "/api/branch-day/start";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let parsed_rate_version = DateTime::parse_from_rfc3339(&input.rate_version)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| {
                V3Error::new(
                    "validationFailed",
                    Some("rateVersion"),
                    "rateVersion must be a valid RFC 3339 timestamp",
                    StatusCode::BAD_REQUEST,
                )
            })?;

        let published_at: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT p.published_at FROM branches b \
             JOIN rate_publications p ON p.id = b.current_publication_id \
             WHERE b.id = $1",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        if !input.equipment_ready
            || !input.rates_verified
            || published_at != Some(parsed_rate_version)
        {
            return Err(V3Error::new(
                "validationFailed",
                None,
                CustodyError::ServiceChecksIncomplete.to_string(),
                StatusCode::BAD_REQUEST,
            ));
        }

        let _ = &self.ports;

        let phase = self
            .service
            .start_service(
                tx,
                who,
                branch,
                StartService {
                    equipment_ready: input.equipment_ready,
                    rates_verified: input.rates_verified,
                    rate_updated: input.rate_version,
                },
                Utc::now(),
            )
            .await
            .map_err(map_service_error)?;

        Ok(DayStartServiceOutput {
            phase: phase.as_str().to_string(),
        })
    }
}

// ------------------------------------------------------------ 2. dayCutoff

pub struct DayCutoffCommand {
    service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>,
}

impl DayCutoffCommand {
    pub fn new(service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>) -> Self {
        Self { service }
    }
}

impl CommandAction for DayCutoffCommand {
    const NAME: &'static str = "dayCutoff";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, Self::NAME);
    type Input = DayCutoffInput;
    type Output = DayCutoffOutput;
    const ROUTE: &'static str = "/api/branch-day/cutoff";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let phase = self
            .service
            .cut_off(tx, who, branch, Utc::now())
            .await
            .map_err(map_service_error)?;

        Ok(DayCutoffOutput {
            phase: phase.as_str().to_string(),
        })
    }
}

// ------------------------------------------------------------ 3. dayTarget

pub struct DayTargetCommand {
    service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>,
}

impl DayTargetCommand {
    pub fn new(service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>) -> Self {
        Self { service }
    }
}

impl CommandAction for DayTargetCommand {
    const NAME: &'static str = "dayTarget";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, Self::NAME);
    type Input = DayTargetInput;
    type Output = DayTargetOutput;
    const ROUTE: &'static str = "/api/branch-day/target";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let mut pairs = Vec::new();
        for (curr_str, amt_str) in &input.amounts {
            let code = CurrencyCode::parse(curr_str).map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("amounts"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;
            let money = Money::parse(amt_str).map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("amounts"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;
            pairs.push((code, money));
        }
        let balances = Balances::of(&pairs);

        self.service
            .set_target(tx, who, branch, balances, Utc::now())
            .await
            .map_err(map_service_error)?;

        Ok(DayTargetOutput { success: true })
    }
}

// ------------------------------------------------------------ 4. clock

pub struct AttendanceClockCommand {
    service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>,
}

impl AttendanceClockCommand {
    pub fn new(service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>) -> Self {
        Self { service }
    }
}

impl CommandAction for AttendanceClockCommand {
    const NAME: &'static str = "clock";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, Self::NAME);
    type Input = AttendanceClockInput;
    type Output = AttendanceClockOutput;
    const ROUTE: &'static str = "/api/attendance";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let dir = match input.direction.as_str() {
            "IN" => ClockDirection::In,
            "OUT" => ClockDirection::Out,
            _ => {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("direction"),
                    "direction must be IN or OUT",
                    StatusCode::BAD_REQUEST,
                ));
            }
        };

        self.service
            .clock(tx, who, branch, dir, Utc::now())
            .await
            .map_err(map_service_error)?;

        Ok(AttendanceClockOutput { success: true })
    }
}

// ------------------------------------------------------------ 5. holdCreate

pub struct CustomerHoldCreateCommand {
    service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>,
}

impl CustomerHoldCreateCommand {
    pub fn new(service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>) -> Self {
        Self { service }
    }
}

impl CommandAction for CustomerHoldCreateCommand {
    const NAME: &'static str = "holdCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, Self::NAME);
    type Input = CustomerHoldCreateInput;
    type Output = CustomerHoldCreateOutput;
    const ROUTE: &'static str = "/api/customer-holds";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let id = self
            .service
            .hold(
                tx,
                who,
                branch,
                &input.customer_id,
                &input.reason,
                Utc::now(),
            )
            .await
            .map_err(map_service_error)?;

        Ok(CustomerHoldCreateOutput { id })
    }
}

// ------------------------------------------------------------ 6. holdRelease

pub struct CustomerHoldReleaseCommand {
    service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>,
}

impl CustomerHoldReleaseCommand {
    pub fn new(service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>) -> Self {
        Self { service }
    }
}

impl CommandAction for CustomerHoldReleaseCommand {
    const NAME: &'static str = "holdRelease";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, Self::NAME);
    type Input = CustomerHoldReleaseInput;
    type Output = CustomerHoldReleaseOutput;
    const ROUTE: &'static str = "/api/customer-holds/{id}/release";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        self.service
            .release(tx, who, branch, &input.id, &input.note, Utc::now())
            .await
            .map_err(map_service_error)?;

        Ok(CustomerHoldReleaseOutput { success: true })
    }
}

// ------------------------------------------------------------ 7. draftBegin

pub struct DraftBeginCommand {
    service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>,
}

impl DraftBeginCommand {
    pub fn new(service: Arc<BranchDayService<PgCustodyRepository, PgShiftRepository>>) -> Self {
        Self { service }
    }
}

impl CommandAction for DraftBeginCommand {
    const NAME: &'static str = "draftBegin";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, Self::NAME);
    type Input = DraftBeginInput;
    type Output = DraftBeginOutput;
    const ROUTE: &'static str = "/api/tx-drafts";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let side = match input.side.as_str() {
            "BUY" => Side::Buy,
            "SELL" => Side::Sell,
            _ => {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("side"),
                    "side must be BUY or SELL",
                    StatusCode::BAD_REQUEST,
                ));
            }
        };
        let currency = CurrencyCode::parse(&input.currency).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("currency"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let amount = Money::parse(&input.amount).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("amount"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;

        let id = self
            .service
            .begin_draft(
                tx,
                who,
                branch,
                BeginDraft {
                    side,
                    currency,
                    amount,
                },
                Utc::now(),
            )
            .await
            .map_err(map_service_error)?;

        Ok(DraftBeginOutput { id })
    }
}

// ------------------------------------------------------------ register & export

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
    let shifts = ShiftService::new(
        Arc::new(
            xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                pool.clone(),
            ),
        ),
        branch_tx,
    )
    .with_lifecycle_hook(super::cash::shift_lifecycle_hook(&pool));
    let repo = Arc::new(PgCustodyRepository::new(pool));
    let ports = Arc::new(PgCustodyPorts);
    let service = Arc::new(BranchDayService::new(repo, shifts, ports.clone()));

    CommandEndpoint::register(
        cfg,
        Arc::new(DayStartServiceCommand::new(service.clone(), ports)),
    );
    CommandEndpoint::register(cfg, Arc::new(DayCutoffCommand::new(service.clone())));
    CommandEndpoint::register(cfg, Arc::new(DayTargetCommand::new(service.clone())));
    CommandEndpoint::register(cfg, Arc::new(AttendanceClockCommand::new(service.clone())));
    CommandEndpoint::register(
        cfg,
        Arc::new(CustomerHoldCreateCommand::new(service.clone())),
    );
    CommandEndpoint::register(
        cfg,
        Arc::new(CustomerHoldReleaseCommand::new(service.clone())),
    );
    CommandEndpoint::register(cfg, Arc::new(DraftBeginCommand::new(service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::branch_day::export_types(dir)
}
