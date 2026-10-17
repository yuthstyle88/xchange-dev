//! RA-2b (#1429): `POST /api/app/rate-alerts` creates a customer rate alert.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::DateTime;
use sqlx::PgPool;
use ts_rs::TS;
use xc_alert::application::rate_alert_service::{
    CreateRateAlert, RateAlertError, RateAlertService,
};
use xc_alert::domain::condition::AlertCondition;
use xc_alert::domain::CustomerSide;
use xc_alert::infrastructure::repository::PgRateAlertRepository;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Bound, CurrencyCode, Sign};
use xc_rate::infrastructure::currency::currency_register::PgCurrencyRegister;
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::rate_alerts::{CreateRateAlertInput, RateAlertOutput};
use crate::presentation::http::v3;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, role_not_allowed_here};
use crate::presentation::http::v3::error::V3Error;

pub(crate) fn make_rate_alert_service(pool: PgPool) -> RateAlertService {
    let repository = Arc::new(PgRateAlertRepository::new(pool.clone()));
    let register = Arc::new(PgCurrencyRegister::new(pool.clone()));
    let published = Arc::new(PgPublishedRates::new(pool));
    RateAlertService::new(repository, register, published)
}

pub(crate) fn map_alert_error(err: RateAlertError) -> V3Error {
    match err {
        RateAlertError::ValidationFailed { reason, message } => {
            let code = reason.unwrap_or("validationFailed");
            V3Error::new(code, None, message, StatusCode::BAD_REQUEST)
        }
        RateAlertError::OutOfScope(msg) => {
            V3Error::new("alertNotFound", None, msg, StatusCode::BAD_REQUEST)
        }
        RateAlertError::StaleVersion(msg) => {
            V3Error::new("staleVersion", None, msg, StatusCode::CONFLICT)
        }
        RateAlertError::IdempotencyConflict(msg) => {
            V3Error::new("idempotencyConflict", None, msg, StatusCode::CONFLICT)
        }
        RateAlertError::Database(msg) | RateAlertError::Internal(msg) => {
            V3Error::new("serverError", None, msg, StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub struct RateAlertCreateCommand {
    pool: PgPool,
}

impl RateAlertCreateCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for RateAlertCreateCommand {
    const NAME: &'static str = "rateAlertCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateAlertCreate");
    type Input = CreateRateAlertInput;
    type Output = RateAlertOutput;
    const ROUTE: &'static str = "/api/app/rate-alerts";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let customer_id = who.customer_id.as_deref().ok_or_else(role_not_allowed_here)?;

        let target =
            v3::input::decimal("target", &input.target, Bound::numeric(20, 8, Sign::Positive))?;

        let currency = CurrencyCode::parse(&input.currency).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("currency"),
                "invalid currency",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let customer_side = CustomerSide::parse(&input.customer_side).ok_or_else(|| {
            V3Error::new(
                "validationFailed",
                Some("customerSide"),
                "invalid customerSide",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let condition = AlertCondition::parse(&input.condition).ok_or_else(|| {
            V3Error::new(
                "validationFailed",
                Some("condition"),
                "invalid condition",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let expires_at = match input.expires_at {
            Some(ref s) => match DateTime::parse_from_rfc3339(s) {
                Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
                Err(_) => {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("expiresAt"),
                        "invalid expiresAt",
                        StatusCode::BAD_REQUEST,
                    ));
                }
            },
            None => None,
        };

        let create_cmd = CreateRateAlert {
            branch: BranchId::new(&input.branch),
            currency,
            customer_side,
            condition,
            target,
            expires_at,
        };

        let service = make_rate_alert_service(self.pool.clone());
        let alert = service
            .create(tx, customer_id, &create_cmd, key.as_str(), chrono::Utc::now())
            .await
            .map_err(map_alert_error)?;

        Ok(RateAlertOutput {
            id: alert.id.to_string(),
            version: i64::from(alert.version),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(RateAlertCreateCommand::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CreateRateAlertInput::export_all(&cfg)?;
    RateAlertOutput::export_all(&cfg)?;
    Ok(())
}
