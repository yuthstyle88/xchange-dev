//! The `admin_customers` view (§7.4, §12.1).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{fixed2, round_cents};

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_customers::{AdminCustomerRow, AdminCustomersFilter};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct AdminCustomersPagedView {
    pool: PgPool,
}

impl AdminCustomersPagedView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}


#[derive(sqlx::FromRow)]
struct CustomerSqlRow {
    id: String,
    name: String,
    nationality: String,
    created_at: DateTime<Utc>,
    member: bool,
    completed: i64,
    volume: String,
    last_at: Option<DateTime<Utc>>,
}

type CustomerCursor = (Option<DateTime<Utc>>, String, DateTime<Utc>, String);

impl PagedView for AdminCustomersPagedView {
    const NAME: &'static str = "admin_customers";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = AdminCustomerRow;
    type Filter = AdminCustomersFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let filter = filter.clone();

        async move {
            let before_cursor: Option<CustomerCursor> =
                if let Some(before_id) = before {
                    let cursor = sqlx::query_as::<_, CustomerCursor>(
                        "SELECT \
                         (SELECT MAX(t.recorded_at) FROM transactions t WHERE t.branch_id = b.id AND t.customer_id = c.id AND t.mode = 'LIVE' AND t.status = 'COMPLETED') AS last_at, \
                         c.display_name AS name, c.created_at, c.id \
                         FROM customers c \
                         JOIN branches b ON b.organization_id = c.organization_id \
                         WHERE b.id = $1 AND c.mode = 'LIVE' AND c.id = $2",
                    )
                    .bind(branch.as_str())
                    .bind(&before_id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(AppError::Database)?;

                    let Some(cursor) = cursor else {
                        return Ok(vec![]);
                    };
                    Some(cursor)
                } else {
                    None
                };

            let mut builder = sqlx::QueryBuilder::new(
                "WITH customer_stats AS ( \
                 SELECT \
                     c.id, \
                     c.display_name AS name, \
                     COALESCE((SELECT ci.nationality FROM customer_identities ci WHERE ci.customer_id = c.id AND ci.type = 'PASSPORT' AND ci.status = 'ACTIVE' LIMIT 1), '') AS nationality, \
                     c.created_at, \
                     EXISTS(SELECT 1 FROM members m WHERE m.branch_id = b.id AND m.customer_id = c.id) AS member, \
                     COUNT(DISTINCT t.id) AS completed, \
                     COALESCE(SUM(tl.thb_amount)::text, '0.00') AS volume, \
                     MAX(t.recorded_at) AS last_at \
                 FROM customers c \
                 JOIN branches b ON b.organization_id = c.organization_id \
                 LEFT JOIN transactions t ON t.branch_id = b.id AND t.customer_id = c.id AND t.mode = 'LIVE' AND t.status = 'COMPLETED' \
                 LEFT JOIN transaction_lines tl ON tl.transaction_id = t.id \
                 WHERE b.id = ",
            );
            builder.push_bind(branch.as_str());
            builder.push(" AND c.mode = 'LIVE'");

            if let Some(ref q) = filter.q
                && !q.is_empty()
            {
                builder.push(" AND (lower(normalize(c.id, NFKC)) LIKE ('%' || replace(replace(replace(lower(normalize(");
                builder.push_bind(q.as_str());
                builder.push(", NFKC)), '\\', '\\\\'), '%', '\\%'), '_', '\\_') || '%') ESCAPE '\\' \
                    OR lower(normalize(c.display_name, NFKC)) LIKE ('%' || replace(replace(replace(lower(normalize(");
                builder.push_bind(q.as_str());
                builder.push(", NFKC)), '\\', '\\\\'), '%', '\\%'), '_', '\\_') || '%') ESCAPE '\\' \
                    OR EXISTS (SELECT 1 FROM customer_identities ci WHERE ci.customer_id = c.id AND ci.type = 'PASSPORT' AND ci.status = 'ACTIVE' AND lower(normalize(ci.nationality, NFKC)) LIKE ('%' || replace(replace(replace(lower(normalize(");
                builder.push_bind(q.as_str());
                builder.push(", NFKC)), '\\', '\\\\'), '%', '\\%'), '_', '\\_') || '%') ESCAPE '\\'))");
            }

            builder.push(
                " GROUP BY c.id, c.display_name, c.created_at, b.id \
                 ) \
                 SELECT id, name, nationality, created_at, member, completed, volume, last_at \
                 FROM customer_stats \
                 WHERE 1=1",
            );

            if let Some((cur_last_at, cur_name, cur_created, cur_id)) = before_cursor {
                if let Some(last_at) = cur_last_at {
                    builder.push(" AND (last_at < ");
                    builder.push_bind(last_at);
                    builder.push(" OR (last_at = ");
                    builder.push_bind(last_at);
                    builder.push(" AND (name, created_at, id) > (");
                    builder.push_bind(cur_name);
                    builder.push(", ");
                    builder.push_bind(cur_created);
                    builder.push(", ");
                    builder.push_bind(cur_id);
                    builder.push(")) OR last_at IS NULL)");
                } else {
                    builder.push(" AND (last_at IS NULL AND (name, created_at, id) > (");
                    builder.push_bind(cur_name);
                    builder.push(", ");
                    builder.push_bind(cur_created);
                    builder.push(", ");
                    builder.push_bind(cur_id);
                    builder.push("))");
                }
            }

            builder.push(" ORDER BY last_at DESC NULLS LAST, name ASC, created_at ASC, id ASC LIMIT ");
            builder.push_bind(limit as i64);

            let query = builder.build_query_as::<CustomerSqlRow>();
            let sql_rows = query.fetch_all(&pool).await.map_err(AppError::Database)?;

            let mut rows = Vec::with_capacity(sql_rows.len());
            for r in sql_rows {
                let volume = format_customer_volume(&r.volume, &r.id)?;

                rows.push(AdminCustomerRow {
                    id: r.id,
                    name: r.name,
                    nationality: r.nationality,
                    created: r
                        .created_at
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    member: r.member,
                    completed: r.completed,
                    volume,
                    last_at: r
                        .last_at
                        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
                });
            }

            Ok(rows)
        }
    }
}

pub(crate) fn format_customer_volume(raw: &str, customer_id: &str) -> Result<String, AppError> {
    let dec = raw.parse::<Decimal>().map_err(|e| {
        AppError::Internal(format!(
            "invalid volume {:?} for customer {}: {e}",
            raw, customer_id
        ))
    })?;
    let cents = round_cents(dec).ok_or_else(|| {
        AppError::Internal(format!(
            "volume {} overflowed round_cents for customer {}",
            raw, customer_id
        ))
    })?;
    Ok(fixed2(cents))
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(cfg, Arc::new(AdminCustomersPagedView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::admin_customers::export_types(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_overflow_past_96_bits_returns_error() {
        // MAX_96 is 2^96 - 1. When cents exceed 96 bits, round_cents returns None.
        let res = format_customer_volume("800000000000000000000000000.00", "CUST-1");
        assert!(
            res.is_err(),
            "expected error for volume overflowing 96 bits of cents"
        );
    }
}
