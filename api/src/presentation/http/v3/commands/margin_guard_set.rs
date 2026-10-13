//! `marginGuardSet` (MK-4, design `marketing-campaigns.md` §4.5, §7).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "margin_guard_set.ts", rename_all = "camelCase")]
pub struct MarginGuardSetInput {
    pub max_spread_share: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "margin_guard_set.ts", rename_all = "camelCase")]
pub struct MarginGuardSetOutput {
    pub max_spread_share: Option<String>,
}

pub struct MarginGuardSetCommand;

impl CommandAction for MarginGuardSetCommand {
    const NAME: &'static str = "marginGuardSet";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "marginGuardSet");
    type Input = MarginGuardSetInput;
    type Output = MarginGuardSetOutput;
    const ROUTE: &'static str = "/api/margin-guard";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let org_id = Uuid::parse_str(&who.organization_id).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("organizationId"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let max_spread_share_dec = match &input.max_spread_share {
            Some(s) => {
                let dec = Decimal::from_str(s).map_err(|e| {
                    V3Error::new(
                        "validationFailed",
                        Some("maxSpreadShare"),
                        e.to_string(),
                        StatusCode::BAD_REQUEST,
                    )
                })?;
                if dec < Decimal::ZERO || dec > Decimal::ONE {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("maxSpreadShare"),
                        "max_spread_share must be between 0 and 1",
                        StatusCode::BAD_REQUEST,
                    ));
                }
                Some(dec)
            }
            None => None,
        };

        let user_id = resolve_user_id(tx, who, org_id).await;

        sqlx::query(
            "INSERT INTO margin_guards (organization_id, max_spread_share, set_by, set_at) \
             VALUES ($1, $2, $3, now()) \
             ON CONFLICT (organization_id) DO UPDATE SET \
                 max_spread_share = EXCLUDED.max_spread_share, \
                 set_by = EXCLUDED.set_by, \
                 set_at = EXCLUDED.set_at",
        )
        .bind(org_id)
        .bind(max_spread_share_dec)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| {
            V3Error::new(
                "database",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        Ok(MarginGuardSetOutput {
            max_spread_share: input.max_spread_share,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(MarginGuardSetCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MarginGuardSetInput::export_all(&cfg)?;
    MarginGuardSetOutput::export_all(&cfg)?;
    Ok(())
}

async fn resolve_user_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    who: &Actor,
    org_id: Uuid,
) -> Uuid {
    if let Ok(u) = Uuid::parse_str(&who.user_id) {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
            .bind(u)
            .fetch_one(&mut **tx)
            .await
            .unwrap_or(false);
        if exists {
            return u;
        }
    }
    let fallback: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE organization_id = $1 LIMIT 1")
            .bind(org_id)
            .fetch_optional(&mut **tx)
            .await
            .unwrap_or(None);
    fallback.unwrap_or_else(|| Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap())
}
