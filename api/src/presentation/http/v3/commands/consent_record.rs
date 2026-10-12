//! `consentRecord`, `POST /api/app/consents` (#654 CI-7, design `customer-identity-consent.md` §4.3, §4.4, K5): a
//! signed-in customer grants or withdraws a consent for themselves, against the newest text of that purpose in the
//! language they read it in. One `consents` row, recorded by their own user, evidence `app:<Idempotency-Key>`.
//!
//! - `400 validationFailed` naming `lang` (not th/en/zh), `purpose`, `channel` (MARKETING without one, TERMS with
//!   one) or `granted` (withdrawing TERMS here is refused, a [proposed] default on #654);
//! - `409 consentTextMissing` (`field: lang`) when no text of that purpose exists in `lang`; nothing is written.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::consent_service::{ConsentChoice, ConsentService};
use xc_customer::domain::consent::{Channel, Purpose};
use xc_kernel::ids::BranchId;

use crate::application::customer_signup::is_consent_text_missing;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

/// The customer account's command types share `my_profile.ts`, the account's one generated file.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_profile.ts")]
pub struct ConsentRecordInput {
    /// `TERMS` or `MARKETING`.
    pub purpose: String,
    /// `SMS`, `EMAIL` or `PUSH`; MARKETING only.
    #[serde(default)]
    pub channel: Option<String>,
    pub granted: bool,
    /// The language the customer read the text in: `th`, `en` or `zh`.
    pub lang: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_profile.ts")]
pub struct ConsentRecordOutput {
    pub purpose: String,
    pub channel: Option<String>,
    pub granted: bool,
    pub policy_version: i32,
    /// RFC 3339.
    pub effective_at: String,
}

pub struct ConsentRecordCommand;

fn invalid(field: &'static str, message: &'static str) -> V3Error {
    V3Error::new(
        "validationFailed",
        Some(field),
        message,
        StatusCode::BAD_REQUEST,
    )
}

impl CommandAction for ConsentRecordCommand {
    const NAME: &'static str = "consentRecord";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "customerSelf");
    type Input = ConsentRecordInput;
    type Output = ConsentRecordOutput;
    const ROUTE: &'static str = "/api/app/consents";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        // The route row passes only a caller with a linked customer; this is that customer.
        let customer = who.customer_id.as_deref().ok_or_else(out_of_scope)?;
        if !["th", "en", "zh"].contains(&input.lang.as_str()) {
            return Err(invalid("lang", "ภาษาต้องเป็น th, en หรือ zh"));
        }
        let purpose: Purpose = input
            .purpose
            .parse()
            .map_err(|_| invalid("purpose", "ประเภทความยินยอมไม่ถูกต้อง"))?;
        let channel: Option<Channel> = match (purpose, input.channel.as_deref()) {
            (Purpose::Marketing, Some(c)) => Some(
                c.parse()
                    .map_err(|_| invalid("channel", "ช่องทางไม่ถูกต้อง"))?,
            ),
            (Purpose::Marketing, None) => {
                return Err(invalid("channel", "ความยินยอมการตลาดต้องระบุช่องทาง"));
            }
            (Purpose::Terms, Some(_)) => {
                return Err(invalid("channel", "ข้อตกลงการใช้งานไม่มีช่องทาง"));
            }
            (Purpose::Terms, None) => None,
        };
        if purpose == Purpose::Terms && !input.granted {
            return Err(invalid("granted", "ถอนความยินยอมข้อตกลงการใช้งานที่นี่ไม่ได้"));
        }

        let now = Utc::now();
        let version = ConsentService::record_in(
            tx,
            customer,
            ConsentChoice {
                purpose,
                channel,
                granted: input.granted,
                lang: input.lang,
                evidence_ref: format!("app:{}", key.as_str()),
                recorded_by: Some(who.user_id.clone()),
            },
            now,
        )
        .await
        .map_err(|e| {
            if is_consent_text_missing(&e) {
                V3Error::new(
                    "consentTextMissing",
                    Some("lang"),
                    "ยังไม่มีข้อความยินยอมในภาษานี้",
                    StatusCode::CONFLICT,
                )
            } else {
                tracing::error!(error = %e, "consentRecord failed");
                server_error()
            }
        })?;

        Ok(ConsentRecordOutput {
            purpose: purpose.as_str().to_owned(),
            channel: channel.map(|c| c.as_str().to_owned()),
            granted: input.granted,
            policy_version: version,
            effective_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(ConsentRecordCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ConsentRecordInput::export_all(&cfg)?;
    ConsentRecordOutput::export_all(&cfg)?;
    Ok(())
}
