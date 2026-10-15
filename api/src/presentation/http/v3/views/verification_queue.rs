//! The `verification_queue` view (§4.5; ticket #1396).
//! `GET /api/v3/views/verification_queue`

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{PiiCipher, Sealed};

use crate::bootstrap::AppState;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint, ViewFilter};

const VIEW_NAME: &str = "verification_queue";

pub struct VerificationQueueView {
    pool: PgPool,
}

impl VerificationQueueView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for VerificationQueueView {
    const NAME: &'static str = VIEW_NAME;
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, VIEW_NAME);
    const VERSIONED: bool = true;
    type Filter = VerificationQueueFilter;
    type Data = Vec<VerificationQueueItem>;

    async fn own_version(&self, _branch: &BranchId, who: &Actor) -> AppResult<Option<i64>> {
        let org_id = match Uuid::parse_str(&who.organization_id) {
            Ok(id) => id,
            Err(_) => return Err(AppError::Forbidden),
        };
        let now = Utc::now();
        let scope = BranchScope::of(who, Permission::VerificationReview, now);
        let branch_list: Option<Vec<String>> = match &scope {
            BranchScope::All => None,
            BranchScope::Branches(branches) => {
                Some(branches.iter().map(|b| b.as_str().to_string()).collect())
            }
        };
        let version: i64 = match &branch_list {
            None => {
                sqlx::query_scalar(
                    "SELECT COALESCE(MAX(EXTRACT(EPOCH FROM updated_at)::bigint), 0) + \
                            COALESCE(SUM(version)::bigint, 0) + \
                            COUNT(*)::bigint + 1 \
                     FROM verification_cases \
                     WHERE organization_id = $1 AND status IN ('SUBMITTED', 'UNDER_REVIEW')",
                )
                .bind(org_id)
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)?
            }
            Some(branches) => {
                sqlx::query_scalar(
                    "SELECT COALESCE(MAX(EXTRACT(EPOCH FROM updated_at)::bigint), 0) + \
                            COALESCE(SUM(version)::bigint, 0) + \
                            COUNT(*)::bigint + 1 \
                     FROM verification_cases \
                     WHERE organization_id = $1 AND branch_id = ANY($2) AND status IN ('SUBMITTED', 'UNDER_REVIEW')",
                )
                .bind(org_id)
                .bind(branches)
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)?
            }
        };
        Ok(Some(version))
    }

    async fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> AppResult<Self::Data> {
        let org_id = match Uuid::parse_str(&who.organization_id) {
            Ok(id) => id,
            Err(_) => return Err(AppError::Forbidden),
        };
        let now = Utc::now();
        let scope = BranchScope::of(who, Permission::VerificationReview, now);
        let branch_list: Option<Vec<String>> = match &scope {
            BranchScope::All => None,
            BranchScope::Branches(branches) => {
                Some(branches.iter().map(|b| b.as_str().to_string()).collect())
            }
        };

        let rows: Vec<sqlx::postgres::PgRow> = match &branch_list {
            None => {
                sqlx::query(
                    "SELECT vc.id, vc.customer_id, vc.status, vc.submitted_at, vc.branch_id, \
                            COALESCE(c.display_name, '') AS customer_display_name \
                     FROM verification_cases vc \
                     LEFT JOIN customers c ON c.id = vc.customer_id AND c.organization_id = vc.organization_id \
                     WHERE vc.organization_id = $1 AND vc.status IN ('SUBMITTED', 'UNDER_REVIEW') \
                     ORDER BY vc.submitted_at ASC, vc.id ASC",
                )
                .bind(org_id)
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?
            }
            Some(branches) => {
                sqlx::query(
                    "SELECT vc.id, vc.customer_id, vc.status, vc.submitted_at, vc.branch_id, \
                            COALESCE(c.display_name, '') AS customer_display_name \
                     FROM verification_cases vc \
                     LEFT JOIN customers c ON c.id = vc.customer_id AND c.organization_id = vc.organization_id \
                     WHERE vc.organization_id = $1 AND vc.branch_id = ANY($2) AND vc.status IN ('SUBMITTED', 'UNDER_REVIEW') \
                     ORDER BY vc.submitted_at ASC, vc.id ASC",
                )
                .bind(org_id)
                .bind(branches)
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?
            }
        };

        let mut last4_by_customer: HashMap<String, String> = HashMap::new();
        if !rows.is_empty()
            && let Some(cipher) = xc_customer::get_pii_keys().ok().map(PiiCipher::new)
        {
            let customer_ids: Vec<String> = rows
                .iter()
                .map(|row| row.get::<String, _>("customer_id"))
                .collect();
            let id_rows: Vec<(String, Vec<u8>, String, String)> = sqlx::query_as(
                "SELECT DISTINCT ON (customer_id) customer_id, encrypted_value, key_id, type \
                 FROM customer_identities \
                 WHERE customer_id = ANY($1) AND organization_id = $2 AND status = 'ACTIVE' \
                 ORDER BY customer_id, created_at DESC",
            )
            .bind(&customer_ids)
            .bind(org_id)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            for (customer_id, enc_val, key_id, id_type) in id_rows {
                let sealed = Sealed {
                    key_id,
                    bytes: enc_val,
                };
                let aad = format!("{customer_id}{id_type}");
                if let Ok(dec) = cipher.decrypt(aad.as_bytes(), &sealed)
                    && let Ok(val) = serde_json::from_slice::<serde_json::Value>(&dec)
                    && let Some(num) = val.get("number").and_then(|n| n.as_str())
                {
                    last4_by_customer.insert(customer_id, last4(num));
                }
            }
        }

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let case_id: Uuid = row.get("id");
            let customer_id: String = row.get("customer_id");
            let status: String = row.get("status");
            let submitted_at: Option<DateTime<Utc>> = row.get("submitted_at");
            let branch_id: Option<String> = row.get("branch_id");
            let customer_display_name: String = row.get("customer_display_name");
            let identity_last4 = last4_by_customer.get(&customer_id).cloned();

            items.push(VerificationQueueItem {
                id: case_id.to_string(),
                status,
                submitted_at: submitted_at.map(|t| t.to_rfc3339()),
                branch_id,
                customer_display_name,
                identity_last4,
            });
        }

        Ok(items)
    }

    fn refusal(&self, err: &AppError) -> Option<V3Error> {
        match err {
            AppError::Forbidden => Some(V3Error::new(
                "outOfScope",
                None,
                "out of scope",
                StatusCode::FORBIDDEN,
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_queue.ts")]
pub struct VerificationQueueFilter {
    #[ts(type = "number | null")]
    pub since: Option<i64>,
}

impl ViewFilter for VerificationQueueFilter {}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_queue.ts")]
pub struct VerificationQueueItem {
    pub id: String,
    pub status: String,
    pub submitted_at: Option<String>,
    pub branch_id: Option<String>,
    pub customer_display_name: String,
    pub identity_last4: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_queue.ts")]
pub struct VerificationQueueAnswer {
    #[ts(type = "number")]
    pub version: i64,
    pub changed: bool,
    pub data: Option<Vec<VerificationQueueItem>>,
}

fn last4(s: &str) -> String {
    match s.char_indices().rev().nth(3) {
        Some((idx, _)) => s[idx..].to_string(),
        None => s.to_string(),
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(VerificationQueueView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    VerificationQueueFilter::export_all(&cfg)?;
    VerificationQueueItem::export_all(&cfg)?;
    VerificationQueueAnswer::export_all(&cfg)?;
    Ok(())
}
