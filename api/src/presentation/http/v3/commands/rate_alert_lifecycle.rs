//! RA-2b (#1429): `POST /api/app/rate-alerts/{id}/pause|resume|delete` lifecycle commands.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_alert::domain::rate_alert::RateAlertId;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use super::rate_alert_create::{make_rate_alert_service, map_alert_error};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::rate_alerts::{RateAlertLifecycleInput, RateAlertOutput};
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, role_not_allowed_here};
use crate::presentation::http::v3::error::V3Error;

fn parse_alert_id(id_opt: Option<&str>) -> Result<RateAlertId, V3Error> {
    let id_str = id_opt.unwrap_or_default();
    match Uuid::parse_str(id_str) {
        Ok(u) => Ok(RateAlertId::new(u)),
        Err(_) => Err(V3Error::new(
            "alertNotFound",
            None,
            "alert not found",
            StatusCode::BAD_REQUEST,
        )),
    }
}

fn validation_failed(field: &'static str, message: impl Into<String>) -> V3Error {
    V3Error::new(
        "validationFailed",
        Some(field),
        message.into(),
        StatusCode::BAD_REQUEST,
    )
}

fn parse_expected_version(v: i64) -> Result<i32, V3Error> {
    if v < 0 {
        return Err(validation_failed(
            "expectedVersion",
            "expectedVersion must be non-negative",
        ));
    }
    i32::try_from(v).map_err(|_| {
        validation_failed(
            "expectedVersion",
            "expectedVersion out of range",
        )
    })
}


pub struct RateAlertPauseCommand {
    pool: PgPool,
}

impl RateAlertPauseCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for RateAlertPauseCommand {
    const NAME: &'static str = "rateAlertPause";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateAlertPause");
    type Input = RateAlertLifecycleInput;
    type Output = RateAlertOutput;
    const ROUTE: &'static str = "/api/app/rate-alerts/{id}/pause";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let customer_id = who.customer_id.as_deref().ok_or_else(role_not_allowed_here)?;
        let alert_id = parse_alert_id(input.id.as_deref())?;
        let expected_version = parse_expected_version(input.expected_version)?;

        let service = make_rate_alert_service(self.pool.clone());
        let alert = service
            .pause(tx, customer_id, alert_id, expected_version, chrono::Utc::now())
            .await
            .map_err(map_alert_error)?;

        Ok(RateAlertOutput {
            id: alert.id.to_string(),
            version: i64::from(alert.version),
        })
    }
}

pub struct RateAlertResumeCommand {
    pool: PgPool,
}

impl RateAlertResumeCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for RateAlertResumeCommand {
    const NAME: &'static str = "rateAlertResume";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateAlertResume");
    type Input = RateAlertLifecycleInput;
    type Output = RateAlertOutput;
    const ROUTE: &'static str = "/api/app/rate-alerts/{id}/resume";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let customer_id = who.customer_id.as_deref().ok_or_else(role_not_allowed_here)?;
        let alert_id = parse_alert_id(input.id.as_deref())?;
        let expected_version = parse_expected_version(input.expected_version)?;

        let service = make_rate_alert_service(self.pool.clone());
        let alert = service
            .resume(tx, customer_id, alert_id, expected_version, chrono::Utc::now())
            .await
            .map_err(map_alert_error)?;

        Ok(RateAlertOutput {
            id: alert.id.to_string(),
            version: i64::from(alert.version),
        })
    }
}

pub struct RateAlertDeleteCommand {
    pool: PgPool,
}

impl RateAlertDeleteCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for RateAlertDeleteCommand {
    const NAME: &'static str = "rateAlertDelete";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateAlertDelete");
    type Input = RateAlertLifecycleInput;
    type Output = RateAlertOutput;
    const ROUTE: &'static str = "/api/app/rate-alerts/{id}/delete";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let customer_id = who.customer_id.as_deref().ok_or_else(role_not_allowed_here)?;
        let alert_id = parse_alert_id(input.id.as_deref())?;
        let expected_version = parse_expected_version(input.expected_version)?;

        let service = make_rate_alert_service(self.pool.clone());
        let alert = service
            .delete(tx, customer_id, alert_id, expected_version, chrono::Utc::now())
            .await
            .map_err(map_alert_error)?;

        Ok(RateAlertOutput {
            id: alert.id.to_string(),
            version: i64::from(alert.version),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(RateAlertPauseCommand::new(state.db.clone())),
    );
    CommandEndpoint::register(
        cfg,
        Arc::new(RateAlertResumeCommand::new(state.db.clone())),
    );
    CommandEndpoint::register(
        cfg,
        Arc::new(RateAlertDeleteCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RateAlertLifecycleInput::export_all(&cfg)?;
    Ok(())
}
