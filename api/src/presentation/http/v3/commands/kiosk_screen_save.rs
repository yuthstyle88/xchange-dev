//! #6273: `POST /api/v3/commands/kiosk_screen_save` — the owner saves a branch's kiosk screen values.
//!
//! EXECUTIVE only (`kioskScreenSave` = `SystemSettings`, seed v10). Versioned and audited like the other
//! `branch_settings` saves: stored under `kiosk_screen::SETTING` through `BranchSettingStore::write_in`, with one
//! audit row per save. The bounds and defaults are `application::kiosk_screen`'s; the refresh period's upper bound
//! is `kiosk_screen::MAX_REFRESH_SECONDS`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde_json::{Value, json};
use sqlx::PgPool;
use ts_rs::TS;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, audit_role};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_kernel::payment::PaymentRules;

use crate::application::kiosk_screen::{self, KioskScreen};
use crate::application::settings::payment_json::rules_from_json;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::presentation::dto::v3::kiosk_screen::{KioskScreenData, KioskScreenSaveInput};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

pub struct KioskScreenSaveCommand {
    pool: PgPool,
}

impl KioskScreenSaveCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn check_range(
    field: &'static str,
    value: Option<i64>,
    range: std::ops::RangeInclusive<u32>,
) -> Result<Option<u32>, V3Error> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let n = u32::try_from(raw)
        .ok()
        .filter(|v| range.contains(v))
        .ok_or_else(|| {
            V3Error::new(
                "validationFailed",
                Some(field),
                format!("{field} out of range"),
                StatusCode::BAD_REQUEST,
            )
        })?;
    Ok(Some(n))
}

fn data_from_screen(branch: &BranchId, screen: KioskScreen, version: Option<i64>) -> KioskScreenData {
    KioskScreenData {
        branch: branch.as_str().to_owned(),
        quick_amounts: screen.quick_amounts,
        tiles: screen.tiles,
        idle_seconds: screen.idle_seconds,
        review_seconds: screen.review_seconds,
        refresh_seconds: screen.refresh_seconds,
        popular_days: screen.popular_days,
        default_payment_method: screen.default_payment_method,
        defaults: screen.defaults.into_iter().map(str::to_owned).collect(),
        version,
    }
}

impl CommandAction for KioskScreenSaveCommand {
    const NAME: &'static str = "kioskScreenSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "kioskScreenSave");
    type Input = KioskScreenSaveInput;
    type Output = KioskScreenData;
    const ROUTE: &'static str = "/api/v3/commands/kiosk_screen_save";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Err(V3Error::from(PolicyError::SignIn))
    }

    async fn run_as(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        caller: &Caller,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let who = caller
            .user
            .as_ref()
            .ok_or_else(|| V3Error::from(PolicyError::SignIn))?;

        let quick_amounts = match input.quick_amounts {
            Some(ref list) => {
                if !(1..=8).contains(&list.len()) {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("quickAmounts"),
                        "quickAmounts must have 1 to 8 items",
                        StatusCode::BAD_REQUEST,
                    ));
                }
                let mut out = Vec::with_capacity(list.len());
                for &raw in list {
                    let n = u32::try_from(raw)
                        .ok()
                        .filter(|v| (1..=10_000_000).contains(v))
                        .ok_or_else(|| {
                            V3Error::new(
                                "validationFailed",
                                Some("quickAmounts"),
                                "quickAmounts items must be positive numbers",
                                StatusCode::BAD_REQUEST,
                            )
                        })?;
                    out.push(n);
                }
                Some(out)
            }
            None => None,
        };

        let tiles = check_range("tiles", input.tiles, 1..=12)?;
        let popular_days = check_range("popularDays", input.popular_days, 1..=365)?;
        let idle_seconds = check_range("idleSeconds", input.idle_seconds, 30..=1800)?;
        let review_seconds = check_range("reviewSeconds", input.review_seconds, 0..=10)?;
        let refresh_seconds = check_range(
            "refreshSeconds",
            input.refresh_seconds,
            5..=kiosk_screen::MAX_REFRESH_SECONDS,
        )?;

        let store = BranchSettingStore::new(self.pool.clone());

        let default_payment_method = match input.default_payment_method {
            Some(ref raw) => {
                let method = raw.trim();
                let rules = match store
                    .read_in(tx, branch, "paymentMethods")
                    .await
                    .map_err(|_| server_error())?
                {
                    Some(s) => rules_from_json(&s.body).unwrap_or_else(PaymentRules::defaults),
                    None => PaymentRules::defaults(),
                };
                if !rules
                    .rule(method)
                    .is_some_and(|r| r.enabled && (r.buy || r.sell))
                {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("defaultPaymentMethod"),
                        "defaultPaymentMethod must be enabled for at least one side",
                        StatusCode::BAD_REQUEST,
                    ));
                }
                Some(method.to_owned())
            }
            None => None,
        };

        let current = store
            .read_in(tx, branch, kiosk_screen::SETTING)
            .await
            .map_err(|_| server_error())?;
        let current_version = current.as_ref().map(|s| s.version);
        if current_version != input.expected_version {
            return Err(V3Error::new(
                "staleVersion",
                None,
                "stale version",
                StatusCode::CONFLICT,
            ));
        }

        let mut map = serde_json::Map::new();
        if let Some(v) = &quick_amounts {
            map.insert("quickAmounts".to_owned(), json!(v));
        }
        if let Some(v) = tiles {
            map.insert("tiles".to_owned(), json!(v));
        }
        if let Some(v) = idle_seconds {
            map.insert("idleSeconds".to_owned(), json!(v));
        }
        if let Some(v) = review_seconds {
            map.insert("reviewSeconds".to_owned(), json!(v));
        }
        if let Some(v) = refresh_seconds {
            map.insert("refreshSeconds".to_owned(), json!(v));
        }
        if let Some(v) = popular_days {
            map.insert("popularDays".to_owned(), json!(v));
        }
        if let Some(v) = &default_payment_method {
            map.insert("defaultPaymentMethod".to_owned(), json!(v));
        }
        let body = Value::Object(map);

        let now = chrono::Utc::now();
        let stored = match store
            .write_in(
                tx,
                branch,
                kiosk_screen::SETTING,
                current_version.unwrap_or(0),
                &json!({}),
                &body,
                &who.user_id,
                now,
            )
            .await
            .map_err(|_| server_error())?
        {
            Ok(s) => s,
            Err(_) => {
                return Err(V3Error::new(
                    "staleVersion",
                    None,
                    "stale version",
                    StatusCode::CONFLICT,
                ));
            }
        };

        let after_data = data_from_screen(
            branch,
            KioskScreen::from_body(&stored.body),
            Some(stored.version),
        );

        let org_id: String =
            sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_one(&mut **tx)
                .await
                .map_err(|_| server_error())?;

        let before = match &current {
            Some(prev) => Some(
                serde_json::to_value(data_from_screen(
                    branch,
                    KioskScreen::from_body(&prev.body),
                    Some(prev.version),
                ))
                .map_err(|_| server_error())?,
            ),
            None => None,
        };
        let after = Some(serde_json::to_value(&after_data).map_err(|_| server_error())?);

        PgAuditLog::new(self.pool.clone())
            .append_in(
                tx,
                &org_id,
                &AuditEvent {
                    actor_id: who.user_id.clone(),
                    actor_role: audit_role(&caller.granted, who, now).to_owned(),
                    scope: "branch".to_owned(),
                    action: "KIOSK_SCREEN_SAVE".to_owned(),
                    object_ref: format!("branch:{}", branch.as_str()),
                    branch_id: Some(branch.as_str().to_owned()),
                    before,
                    after,
                },
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "kiosk screen save audit failed");
                server_error()
            })?;

        Ok(after_data)
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(KioskScreenSaveCommand::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    KioskScreenSaveInput::export_all(&cfg)?;
    Ok(())
}
