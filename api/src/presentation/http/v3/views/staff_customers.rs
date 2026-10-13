//! The `staff_customers` view (§3.1).

use std::future::Future;
use std::path::Path;

use actix_web::web;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher};

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::staff_customers::{StaffCustomerRow, StaffCustomersFilter};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

pub struct StaffCustomersPagedView {
    pool: PgPool,
}

impl StaffCustomersPagedView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PagedView for StaffCustomersPagedView {
    const NAME: &'static str = "staff_customers";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "staffCustomers");
    type Row = StaffCustomerRow;
    type Filter = StaffCustomersFilter;

    fn page(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
        _before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let who = who.clone();
        let branch = branch.clone();
        let q_opt = filter.q.clone();

        async move {
            let Some(raw_q) = q_opt else {
                return Ok(vec![]);
            };
            let q = raw_q.trim();
            if q.is_empty() {
                return Ok(vec![]);
            }

            let org_id: Uuid = if let Ok(org_uuid) = Uuid::parse_str(&who.organization_id) {
                org_uuid
            } else {
                // stopgap (#1463): old-cookie actors still name their organisation by code until #1463 merges; remove this branch then (tracked in #1518).
                let org: Option<Uuid> =
                    sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                        .bind(branch.as_str())
                        .fetch_optional(&pool)
                        .await
                        .map_err(AppError::Database)?;
                org.ok_or_else(|| AppError::NotFound {
                    entity: "branch",
                    id: branch.to_string(),
                })?
            };

            let mut matched_customer_ids: Vec<String> = Vec::new();

            let pii_keys =
                xc_customer::get_pii_keys().map_err(|e| AppError::Internal(e.to_string()))?;
            let nationalities: Vec<String> = sqlx::query_scalar(
                "SELECT DISTINCT nationality FROM customer_identities \
                 WHERE organization_id = $1 AND type = 'PASSPORT' AND status = 'ACTIVE' AND nationality IS NOT NULL",
            )
            .bind(org_id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let mut hashes = Vec::new();
            for nat in nationalities {
                if let Ok(normalized) =
                    xc_kernel::pii::normalize(IdentityType::Passport, q, Some(&nat))
                {
                    let hash =
                        LookupHasher::new(&pii_keys).hash(IdentityType::Passport, &normalized);
                    hashes.push(hash);
                }
            }

            if !hashes.is_empty() {
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT DISTINCT customer_id FROM customer_identities \
                     WHERE organization_id = $1 AND type = 'PASSPORT' AND status = 'ACTIVE' AND lookup_hash = ANY($2)",
                )
                .bind(org_id)
                .bind(&hashes)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?;

                matched_customer_ids.extend(ids);
            }

            let max_limit = if limit == 0 { 50 } else { limit.clamp(1, 100) } as i64;

            let rows = sqlx::query(
                "SELECT id, display_name FROM customers \
                 WHERE organization_id = $1 AND ( \
                     LOWER(id) = LOWER($2) \
                     OR LOWER(display_name) = LOWER($2) \
                     OR id = ANY($3) \
                 ) \
                 ORDER BY display_name, id \
                 LIMIT $4",
            )
            .bind(org_id)
            .bind(q)
            .bind(&matched_customer_ids)
            .bind(max_limit)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let result: Vec<StaffCustomerRow> = rows
                .into_iter()
                .map(|r| StaffCustomerRow {
                    id: r.get("id"),
                    display_name: r.get("display_name"),
                })
                .collect();

            Ok(result)
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        std::sync::Arc::new(StaffCustomersPagedView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::staff_customers::export_types(dir)
}
