//! `profileUpdate`, `PUT /api/app/profile` (#654 CI-7, design `customer-identity-consent.md` §4.3, §4.4): a
//! signed-in customer changes their own display name and language. The body names no customer: it always acts on
//! the caller's (`deny_unknown_fields`, so a `customerId` is `400 invalidRequest`).
//!
//! - `400 validationFailed`: neither field given, a blank `displayName`, or `preferredLanguage` not th/en/zh;
//! - `400 nameTooLong` (`field: displayName`): `displayName` over 100 characters after trim (#4397);
//! - `409 consentTextMissing` (`field: preferredLanguage`): a language with no TERMS text (§4.3: a customer screen
//!   offers a language only when its consent texts exist). Nothing is changed.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::consent_service::ConsentService;
use xc_customer::domain::consent::Purpose;
use xc_customer::domain::customer::value_objects::exceeds_display_name_limit;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

/// The customer account's command types share `my_profile.ts`, the account's one generated file.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "my_profile.ts")]
pub struct ProfileUpdateInput {
    #[serde(default)]
    pub display_name: Option<String>,
    /// `th`, `en` or `zh`.
    #[serde(default)]
    pub preferred_language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_profile.ts")]
pub struct ProfileUpdateOutput {
    pub customer_id: String,
    pub display_name: String,
    pub preferred_language: String,
}

pub struct ProfileUpdateCommand;

fn invalid(field: Option<&'static str>, message: &'static str) -> V3Error {
    V3Error::new("validationFailed", field, message, StatusCode::BAD_REQUEST)
}

impl CommandAction for ProfileUpdateCommand {
    const NAME: &'static str = "profileUpdate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "customerSelf");
    type Input = ProfileUpdateInput;
    type Output = ProfileUpdateOutput;
    const ROUTE: &'static str = "/api/app/profile";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        // The route row passes only a caller with a linked customer; this is that customer.
        let customer = who.customer_id.as_deref().ok_or_else(out_of_scope)?;
        if input.display_name.is_none() && input.preferred_language.is_none() {
            return Err(invalid(None, "กรุณาระบุชื่อหรือภาษา"));
        }
        let name = input.display_name.as_deref().map(str::trim);
        if name.is_some_and(str::is_empty) {
            return Err(invalid(Some("displayName"), "กรุณากรอกชื่อ"));
        }
        if name.is_some_and(exceeds_display_name_limit) {
            return Err(V3Error::new(
                "nameTooLong",
                Some("displayName"),
                "ชื่อที่แสดงยาวได้ไม่เกิน 100 ตัวอักษร",
                StatusCode::BAD_REQUEST,
            ));
        }
        let lang = input.preferred_language.as_deref();
        if let Some(lang) = lang {
            if !["th", "en", "zh"].contains(&lang) {
                return Err(invalid(
                    Some("preferredLanguage"),
                    "ภาษาต้องเป็น th, en หรือ zh",
                ));
            }
            let org: String =
                sqlx::query_scalar("SELECT organization_id::text FROM customers WHERE id = $1")
                    .bind(customer)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(|_| server_error())?;
            let terms = ConsentService::current_version_in(tx, &org, Purpose::Terms, lang)
                .await
                .map_err(|_| server_error())?;
            if terms.is_none() {
                return Err(V3Error::new(
                    "consentTextMissing",
                    Some("preferredLanguage"),
                    "ยังไม่มีข้อความยินยอมในภาษานี้",
                    StatusCode::CONFLICT,
                ));
            }
        }

        let row: (String, String, String) = sqlx::query_as(
            "UPDATE customers SET display_name = COALESCE($2, display_name), \
                 preferred_language = COALESCE($3, preferred_language), updated_at = $4 \
             WHERE id = $1 RETURNING id, display_name, preferred_language",
        )
        .bind(customer)
        .bind(name)
        .bind(lang)
        .bind(Utc::now())
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "profileUpdate failed");
            server_error()
        })?;

        Ok(ProfileUpdateOutput {
            customer_id: row.0,
            display_name: row.1,
            preferred_language: row.2,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(ProfileUpdateCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ProfileUpdateInput::export_all(&cfg)?;
    ProfileUpdateOutput::export_all(&cfg)?;
    Ok(())
}
