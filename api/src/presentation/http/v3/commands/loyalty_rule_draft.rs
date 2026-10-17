//! LY-6 (#1411, design `membership-points.md` §3, §4.6, §6.4): `loyaltyRuleDraft` command.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_loyalty::application::rule_publisher::{LoyaltyRuleError, LoyaltyRuleWriter, RuleKind};
use xc_loyalty::domain::customer_ref::NS_XC_LOYALTY;
use xc_loyalty::infrastructure::http_rule_writer::HttpLoyaltyRuleWriter;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "loyalty_rule_draft.ts")]
pub struct LoyaltyRuleDraftInput {
    pub kind: String,
    #[ts(type = "Record<string, unknown>")]
    pub fields: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "loyalty_rule_draft.ts")]
pub struct LoyaltyRuleDraftOutput {
    pub version_id: String,
    pub status: String,
}

pub struct LoyaltyRuleDraftCommand {
    writer: Option<Arc<dyn LoyaltyRuleWriter>>,
}

impl LoyaltyRuleDraftCommand {
    pub fn new(writer: Option<Arc<dyn LoyaltyRuleWriter>>) -> Self {
        Self { writer }
    }
}

/// Deterministic draft UUID derived from `(kind, branch, key)` so a retry after a lost response
/// targets the same Loyalty rule version (`#2947`).
pub fn derive_draft_id(kind: RuleKind, branch: &BranchId, key: &IdempotencyKey) -> Uuid {
    let name = format!("loyalty-rule-draft:{}:{branch}:{key}", kind.route_segment());
    Uuid::new_v5(&NS_XC_LOYALTY, name.as_bytes())
}

fn loyalty_unavailable(message: impl Into<String>) -> V3Error {
    V3Error::new(
        "loyaltyUnavailable",
        None,
        message,
        StatusCode::SERVICE_UNAVAILABLE,
    )
}

fn map_loyalty_error(err: LoyaltyRuleError) -> V3Error {
    tracing::warn!(error = %err, "loyalty rule draft error");
    let message = err.client_message();
    match err {
        LoyaltyRuleError::Unavailable(_) => loyalty_unavailable(message),
        LoyaltyRuleError::Invalid(_) => V3Error::new(
            "validationFailed",
            Some("fields"),
            message,
            StatusCode::BAD_REQUEST,
        ),
        LoyaltyRuleError::NotFound => {
            V3Error::new("notFound", None, message, StatusCode::NOT_FOUND)
        }
        LoyaltyRuleError::InvalidTransition(_) => {
            V3Error::new("staleVersion", None, message, StatusCode::CONFLICT)
        }
    }
}

impl CommandAction for LoyaltyRuleDraftCommand {
    const NAME: &'static str = "loyaltyRuleDraft";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "loyaltyRuleDraft");
    type Input = LoyaltyRuleDraftInput;
    type Output = LoyaltyRuleDraftOutput;
    const ROUTE: &'static str = "/api/v3/commands/loyalty_rule_draft";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        _who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let Some(writer) = &self.writer else {
            return Err(loyalty_unavailable(
                "loyalty service token is not configured",
            ));
        };

        let Some(kind) = RuleKind::parse(&input.kind) else {
            return Err(V3Error::new(
                "validationFailed",
                Some("kind"),
                "kind must be POINT or TIER",
                StatusCode::BAD_REQUEST,
            ));
        };

        let Some(fields_map) = input.fields.as_object() else {
            return Err(V3Error::new(
                "validationFailed",
                Some("fields"),
                "fields must be a JSON object",
                StatusCode::BAD_REQUEST,
            ));
        };

        let mut fields_map = fields_map.clone();
        let derived_id = if matches!(fields_map.get("id"), None | Some(Value::Null)) {
            let id = derive_draft_id(kind, branch, &key);
            fields_map.insert("id".to_owned(), Value::String(id.to_string()));
            Some(id)
        } else {
            None
        };

        let created = match writer.create_draft(kind, &fields_map).await {
            Ok(v) => v,
            Err(err @ LoyaltyRuleError::InvalidTransition(_)) => {
                if let Some(id) = derived_id {
                    match writer.get(kind, id).await {
                        Ok(Some(v)) => v,
                        Ok(None) => return Err(map_loyalty_error(err)),
                        Err(get_err) => return Err(map_loyalty_error(get_err)),
                    }
                } else {
                    return Err(map_loyalty_error(err));
                }
            }
            Err(err) => return Err(map_loyalty_error(err)),
        };

        Ok(LoyaltyRuleDraftOutput {
            version_id: created.id.to_string(),
            status: created.status,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let writer = state
        .settings
        .loyalty
        .as_ref()
        .and_then(HttpLoyaltyRuleWriter::from_config)
        .map(|w| Arc::new(w) as Arc<dyn LoyaltyRuleWriter>);
    CommandEndpoint::register(cfg, Arc::new(LoyaltyRuleDraftCommand::new(writer)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    LoyaltyRuleDraftInput::export_all(&cfg)?;
    LoyaltyRuleDraftOutput::export_all(&cfg)?;
    Ok(())
}
