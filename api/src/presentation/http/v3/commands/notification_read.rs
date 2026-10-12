//! Notification read commands (§4.4, §6).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_notify::application::notification_service::NotificationService;
use xc_notify::application::template_registry::TemplateRegistry;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "notification_read.ts")]
pub struct NotificationReadInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "notification_read.ts")]
pub struct NotificationReadOutput {
    pub read_at: String,
}

pub struct NotificationReadCommand {
    #[allow(dead_code)]
    service: Arc<NotificationService>,
}

impl NotificationReadCommand {
    pub fn new(service: Arc<NotificationService>) -> Self {
        Self { service }
    }
}

impl CommandAction for NotificationReadCommand {
    const NAME: &'static str = "notificationRead";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "notificationRead");
    type Input = NotificationReadInput;
    type Output = NotificationReadOutput;
    const ROUTE: &'static str = "/api/app/notifications/{id}/read";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let user_uuid = Uuid::parse_str(&who.user_id).map_err(|_| server_error())?;
        let notif_uuid = Uuid::parse_str(&input.id)
            .map_err(|_| V3Error::new("notFound", None, "not found", StatusCode::NOT_FOUND))?;

        let row: Option<(Uuid, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT recipient_user_id, read_at FROM notifications WHERE id = $1",
        )
        .bind(notif_uuid)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to query notification");
            server_error()
        })?;

        let (recipient, read_at) = row.ok_or_else(|| {
            V3Error::new("notFound", None, "not found", StatusCode::NOT_FOUND)
        })?;

        if recipient != user_uuid {
            return Err(V3Error::new(
                "outOfScope",
                None,
                "out of scope",
                StatusCode::FORBIDDEN,
            ));
        }

        if let Some(existing) = read_at {
            return Ok(NotificationReadOutput {
                read_at: existing.to_rfc3339(),
            });
        }

        let updated: DateTime<Utc> = sqlx::query_scalar(
            "UPDATE notifications SET read_at = now() \
             WHERE id = $1 AND read_at IS NULL \
             RETURNING read_at",
        )
        .bind(notif_uuid)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to update notification read_at");
            server_error()
        })?;

        Ok(NotificationReadOutput {
            read_at: updated.to_rfc3339(),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "notification_read.ts")]
pub struct NotificationReadAllInput {}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "notification_read.ts")]
pub struct NotificationReadAllOutput {
    #[ts(type = "number")]
    pub count: u64,
}

pub struct NotificationReadAllCommand {
    #[allow(dead_code)]
    service: Arc<NotificationService>,
}

impl NotificationReadAllCommand {
    pub fn new(service: Arc<NotificationService>) -> Self {
        Self { service }
    }
}

impl CommandAction for NotificationReadAllCommand {
    const NAME: &'static str = "notificationReadAll";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "notificationReadAll");
    type Input = NotificationReadAllInput;
    type Output = NotificationReadAllOutput;
    const ROUTE: &'static str = "/api/app/notifications/read-all";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let user_uuid = Uuid::parse_str(&who.user_id).map_err(|_| server_error())?;

        let res = sqlx::query(
            "UPDATE notifications SET read_at = now() \
             WHERE recipient_user_id = $1 AND read_at IS NULL",
        )
        .bind(user_uuid)
        .execute(&mut **tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to mark all notifications read");
            server_error()
        })?;

        Ok(NotificationReadAllOutput {
            count: res.rows_affected(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let service = Arc::new(NotificationService::new(
        state.db.clone(),
        Arc::new(TemplateRegistry::generated()),
    ));
    CommandEndpoint::register(cfg, Arc::new(NotificationReadCommand::new(service.clone())));
    CommandEndpoint::register(cfg, Arc::new(NotificationReadAllCommand::new(service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    NotificationReadInput::export_all(&cfg)?;
    NotificationReadOutput::export_all(&cfg)?;
    NotificationReadAllInput::export_all(&cfg)?;
    NotificationReadAllOutput::export_all(&cfg)?;
    Ok(())
}
