//! Consent text publish command (§4.4, §6).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::consent_service::ConsentService;
use xc_customer::domain::consent::Purpose;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input::{self, TextBound};

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "consent_text_publish.ts")]
pub struct ConsentTextPublishInput {
    pub purpose: String,
    pub lang: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "consent_text_publish.ts")]
pub struct ConsentTextPublishOutput {
    pub purpose: String,
    pub lang: String,
    pub version: i32,
}

pub struct ConsentTextPublishCommand;

impl CommandAction for ConsentTextPublishCommand {
    const NAME: &'static str = "consentTextPublish";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "consentPolicy");
    type Input = ConsentTextPublishInput;
    type Output = ConsentTextPublishOutput;
    const ROUTE: &'static str = "/api/consent-texts";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let purpose: Purpose = input.purpose.parse().map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("purpose"),
                "purpose must be TERMS or MARKETING",
                StatusCode::BAD_REQUEST,
            )
        })?;

        if input.lang != "th" && input.lang != "en" && input.lang != "zh" {
            return Err(V3Error::new(
                "validationFailed",
                Some("lang"),
                "lang must be th, en, or zh",
                StatusCode::BAD_REQUEST,
            ));
        }

        input::text(
            "body",
            &input.body,
            TextBound {
                min: 1,
                max: 20_000,
            },
        )?;

        let org_id: Option<Uuid> =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|_| server_error())?;

        let org_id = org_id.ok_or_else(server_error)?;

        let pub_by = Uuid::parse_str(&who.user_id).ok().map(|u| u.to_string());
        let now = Utc::now();

        let text = ConsentService::publish_in(
            tx,
            &org_id.to_string(),
            pub_by.as_deref(),
            purpose,
            &input.lang,
            &input.body,
            now,
        )
        .await
        .map_err(|_| server_error())?;

        Ok(ConsentTextPublishOutput {
            purpose: text.purpose.to_string(),
            lang: text.lang,
            version: text.version,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(ConsentTextPublishCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ConsentTextPublishInput::export_all(&cfg)?;
    ConsentTextPublishOutput::export_all(&cfg)?;
    Ok(())
}
