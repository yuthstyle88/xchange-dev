//! The `cashFromBrowser` branch setting (`docs/design/xc-counter-app.md` §5.4, §8.4).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;

use crate::infrastructure::device_agents::counter_policy::CASH_FROM_BROWSER;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::infrastructure::settings::stored_setting::{StoredSetting, VersionConflict};

/// Reads the branch's `cashFromBrowser` setting (`(value, version)`), answering `(None, 0)` when there is no row.
pub async fn read_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
) -> AppResult<(Option<bool>, i64)> {
    let row: Option<(i64, String)> = sqlx::query_as(
        "SELECT version, body::text \
         FROM branch_settings \
         WHERE branch_id = $1 AND name = $2",
    )
    .bind(branch.as_str())
    .bind(CASH_FROM_BROWSER)
    .fetch_optional(&mut **tx)
    .await
    .map_err(AppError::Database)?;

    match row {
        Some((version, body_str)) => {
            let body: Value = serde_json::from_str(&body_str)
                .map_err(|e| AppError::Internal(format!("corrupt setting body: {e}")))?;
            let value = body.get("value").and_then(Value::as_bool);
            Ok((value, version))
        }
        None => Ok((None, 0)),
    }
}

/// Saves `cashFromBrowser` through [`BranchSettingStore::write_in`] with body `{"value": true|false|null}`.
pub async fn save(
    store: &BranchSettingStore,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    expected_version: i64,
    value: Option<bool>,
    actor: &str,
    now: DateTime<Utc>,
) -> AppResult<Result<StoredSetting, VersionConflict>> {
    let first_before = json!({ "value": null });
    let body = json!({ "value": value });
    store
        .write_in(
            tx,
            branch,
            CASH_FROM_BROWSER,
            expected_version,
            &first_before,
            &body,
            actor,
            now,
        )
        .await
}
