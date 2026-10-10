//! Notification read commands (§4.4, §6).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::notifications::NotificationId;
use xc_notify::application::notification_service::NotificationService;
use xc_notify::application::template_registry::TemplateRegistry;
use xc_notify::domain::error::NotifyError;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

fn map_notify_error(err: NotifyError) -> V3Error {
    match err {
        NotifyError::NotFound => {
            V3Error::new("notFound", None, "not found", StatusCode::NOT_FOUND)
        }
        NotifyError::OutOfScope => {
            V3Error::new("outOfScope", None, "out of scope", StatusCode::FORBIDDEN)
        }
        NotifyError::Store(_) => server_error(),
    }
}

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
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let notif_id = NotificationId(input.id);
        let read_at = self
            .service
            .mark_read(who.id(), &notif_id)
            .await
            .map_err(map_notify_error)?;

        Ok(NotificationReadOutput {
            read_at: read_at.to_rfc3339(),
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
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let count = self
            .service
            .mark_all_read(who.id())
            .await
            .map_err(map_notify_error)?;

        Ok(NotificationReadAllOutput { count })
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
