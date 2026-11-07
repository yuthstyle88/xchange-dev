//! PostgreSQL `CounterStore` (xc-counter-app.md §8.4). CA-1d fills the bodies.

use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use super::agent_repository::DeviceAgent;
use super::counter_store::{CounterSettings, CounterStore};
use super::pg_agent_repository::{AGENT_COLUMNS, AgentRow, PgAgentRepository};

impl CounterStore for PgAgentRepository {
    async fn counters(&self, branch: &BranchId) -> AppResult<Vec<DeviceAgent>> {
        let rows: Vec<AgentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {AGENT_COLUMNS} FROM device_agents \
             WHERE branch_id = $1 AND revoked_at IS NULL \
             ORDER BY created_at, id"
        )))
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(DeviceAgent::from).collect())
    }

    async fn save_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        agent_id: &str,
        settings: &CounterSettings,
    ) -> AppResult<Option<DeviceAgent>> {
        if uuid::Uuid::parse_str(agent_id).is_err() {
            return Ok(None);
        }
        let row: Option<AgentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE device_agents \
             SET counter_name = $3, cash_drawer = $4, scanner = $5, kiosk_lock = $6 \
             WHERE branch_id = $1 AND id = $2::uuid AND revoked_at IS NULL \
             RETURNING {AGENT_COLUMNS}"
        )))
        .bind(branch.as_str())
        .bind(agent_id)
        .bind(settings.name.trim())
        .bind(settings.cash_drawer)
        .bind(settings.scanner)
        .bind(settings.kiosk_lock)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(DeviceAgent::from))
    }
}
