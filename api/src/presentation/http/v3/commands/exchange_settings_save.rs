//! TX-4 #672: the two exchange settings commands (design `quote-transaction.md` §4.2, §3.6):
//! `PUT /api/exchange-settings` (the organisation's quote lifetime, `SystemSettings`, T1) and
//! `PUT /api/branches/{id}/exchange-limit` (a branch's approval limit, `BranchesManage`, T2).
//! Each writes one AP-1 `AuditLog` row with the stored value before and after (§3.6).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::access::Standing;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::domain::settings::repository::SettingsRepository;
use xc_exchange::infrastructure::settings::repository::PgSettingsRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;

use super::transactions::refusal;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::exchange_settings::{
    BranchExchangeLimitDto, ExchangeSettingsDto,
};
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, Locks, out_of_scope, role_not_allowed_here, server_error,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "exchange_settings.ts")]
pub struct ExchangeSettingsInput {
    /// 30–3600 (§3.6).
    pub quote_ttl_seconds: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "exchange_settings.ts")]
pub struct BranchExchangeLimitInput {
    /// The path's `{id}`: the branch.
    pub id: String,
    /// A 2-place THB amount above zero; `null` clears the limit, so no exchange waits (T2).
    pub approval_limit_thb: Option<String>,
}

/// The organisation of the branch the command runs at.
async fn organization_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
) -> Result<Uuid, V3Error> {
    let org: Option<Uuid> =
        sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| server_error())?;
    org.ok_or_else(|| {
        V3Error::new(
            "notFound",
            Some("branch"),
            "no such branch",
            StatusCode::NOT_FOUND,
        )
    })
}

async fn audit(
    log: &PgAuditLog,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: AuditEvent,
) -> Result<(), V3Error> {
    log.append(tx, &event).await.map(|_| ()).map_err(|e| {
        tracing::error!(error = %e, "exchange settings audit failed");
        server_error()
    })
}

// ------------------------------------------------------------ PUT /api/exchange-settings

pub struct ExchangeSettingsSaveCommand {
    settings: PgSettingsRepository,
    audit: PgAuditLog,
}

impl ExchangeSettingsSaveCommand {
    pub fn new(pool: PgPool) -> Self {
        Self {
            settings: PgSettingsRepository::new(pool.clone()),
            audit: PgAuditLog::new(pool),
        }
    }
}

impl CommandAction for ExchangeSettingsSaveCommand {
    const NAME: &'static str = "exchangeSettingsSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "exchangeSettingsSave");
    type Input = ExchangeSettingsInput;
    type Output = ExchangeSettingsDto;
    const ROUTE: &'static str = "/api/exchange-settings";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        if BranchScope::of(who, Permission::SystemSettings, Utc::now()) != BranchScope::All {
            return Err(out_of_scope());
        }
        let standing = Standing::of(who, Utc::now()).ok_or_else(role_not_allowed_here)?;
        let org = organization_of(tx, branch).await?;
        let before = self
            .settings
            .get_ttl_in(tx, org)
            .await
            .map_err(refusal)?;
        self.settings
            .set_ttl_in(tx, org, input.quote_ttl_seconds, &who.user_id, Utc::now())
            .await
            .map_err(refusal)?;
        let after = self
            .settings
            .get_ttl_in(tx, org)
            .await
            .map_err(refusal)?;
        audit(
            &self.audit,
            tx,
            AuditEvent {
                actor_id: who.user_id.clone(),
                actor_role: standing.kind.audit_text().to_owned(),
                scope: "organization".to_owned(),
                action: Self::NAME.to_owned(),
                object_ref: org.to_string(),
                branch_id: None,
                before: Some(json!({ "quoteTtlSeconds": before })),
                after: Some(json!({ "quoteTtlSeconds": after })),
            },
        )
        .await?;
        Ok(ExchangeSettingsDto {
            quote_ttl_seconds: after,
        })
    }
}

// ------------------------------------------------------------ PUT /api/branches/{id}/exchange-limit

pub struct BranchExchangeLimitSaveCommand {
    settings: PgSettingsRepository,
    audit: PgAuditLog,
}

impl BranchExchangeLimitSaveCommand {
    pub fn new(pool: PgPool) -> Self {
        Self {
            settings: PgSettingsRepository::new(pool.clone()),
            audit: PgAuditLog::new(pool),
        }
    }
}

impl CommandAction for BranchExchangeLimitSaveCommand {
    const NAME: &'static str = "branchExchangeLimitSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "branchExchangeLimitSave");
    type Input = BranchExchangeLimitInput;
    type Output = BranchExchangeLimitDto;
    const ROUTE: &'static str = "/api/branches/{id}/exchange-limit";
    const METHOD: Method = Method::PUT;

    /// The branch the path names: a staff caller naming another branch is out of scope.
    fn locks(&self, _branch: &BranchId, input: &Self::Input) -> Locks {
        Locks::One(BranchId::new(input.id.clone()))
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let standing = Standing::of(who, Utc::now()).ok_or_else(role_not_allowed_here)?;
        let target = BranchId::new(input.id.clone());
        let limit = match input.approval_limit_thb.as_deref() {
            None => None,
            Some(text) => Some(Money::parse(text).map_err(|_| {
                V3Error::new(
                    "validationFailed",
                    Some("approvalLimitThb"),
                    "approvalLimitThb must be a THB amount",
                    StatusCode::BAD_REQUEST,
                )
            })?),
        };
        let org = organization_of(tx, &target).await?;
        let before = self
            .settings
            .approval_limit_in(tx, org, &target)
            .await
            .map_err(refusal)?;
        self.settings
            .set_approval_limit_in(tx, org, &target, limit, &who.user_id, Utc::now())
            .await
            .map_err(refusal)?;
        let after = self
            .settings
            .approval_limit_in(tx, org, &target)
            .await
            .map_err(refusal)?;
        let text = |m: Option<Money>| m.map(|m| m.to_string());
        audit(
            &self.audit,
            tx,
            AuditEvent {
                actor_id: who.user_id.clone(),
                actor_role: standing.kind.audit_text().to_owned(),
                scope: "branch".to_owned(),
                action: Self::NAME.to_owned(),
                object_ref: target.as_str().to_owned(),
                branch_id: Some(target.as_str().to_owned()),
                before: Some(json!({ "approvalLimitThb": text(before) })),
                after: Some(json!({ "approvalLimitThb": text(after) })),
            },
        )
        .await?;
        Ok(BranchExchangeLimitDto {
            branch_id: target.as_str().to_owned(),
            approval_limit_thb: text(after),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(ExchangeSettingsSaveCommand::new(state.db.clone())),
    );
    CommandEndpoint::register(
        cfg,
        Arc::new(BranchExchangeLimitSaveCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ExchangeSettingsInput::export_all(&cfg)?;
    BranchExchangeLimitInput::export_all(&cfg)?;
    ExchangeSettingsDto::export_all(&cfg)?;
    BranchExchangeLimitDto::export_all(&cfg)?;
    Ok(())
}
