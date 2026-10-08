//! Booking rules application service (T15-e, design §5.6.2, §5.6.5).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;

pub const SETTING_NAME: &str = "bookingRules";

fn parse_rules(body: &Value) -> AppResult<BookingRules> {
    let obj = body.as_object().ok_or_else(|| {
        AppError::Internal("corrupt booking rules: not a JSON object".to_string())
    })?;
    let enabled = obj.get("enabled").unwrap_or(&Value::Null);
    let minutes = obj.get("minutes").unwrap_or(&Value::Null);
    BookingRules::parse(enabled, minutes)
        .map_err(|e| AppError::Internal(format!("corrupt booking rules: {e}")))
}

#[derive(Clone)]
pub struct BookingRulesService {
    branch_tx: BranchTx,
    store: BranchSettingStore,
}

impl BookingRulesService {
    pub fn new(branch_tx: BranchTx, store: BranchSettingStore) -> Self {
        Self { branch_tx, store }
    }

    /// Reads booking rules for a branch (returns None if no row).
    pub async fn rules(&self, branch: &BranchId) -> AppResult<Option<BookingRules>> {
        let stored = self.store.read(branch, SETTING_NAME).await?;
        match stored {
            Some(s) => parse_rules(&s.body).map(Some),
            None => Ok(None),
        }
    }

    /// Reads booking rules for a branch inside an existing transaction (returns None if no row).
    pub async fn rules_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
    ) -> AppResult<Option<BookingRules>> {
        let stored = self.store.read_in(tx, branch, SETTING_NAME).await?;
        match stored {
            Some(s) => parse_rules(&s.body).map(Some),
            None => Ok(None),
        }
    }

    /// Saves booking rules inside the branch's transaction (last write wins, writes history row).
    pub async fn save(
        &self,
        branch: &BranchId,
        rules: BookingRules,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let store = self.store.clone();
        let branch_clone = branch.clone();
        let actor_str = actor.to_string();
        let body = json!({
            "enabled": rules.enabled,
            "minutes": rules.minutes,
        });

        self.branch_tx
            .run(branch, move |tx| {
                let body = body.clone();
                let actor_str = actor_str.clone();
                let branch_clone = branch_clone.clone();
                let store = store.clone();
                Box::pin(async move {
                    let current = store.read_in(tx, &branch_clone, SETTING_NAME).await?;
                    let expected_version = current.as_ref().map_or(0, |s| s.version);
                    let result = store
                        .write_in(
                            tx,
                            &branch_clone,
                            SETTING_NAME,
                            expected_version,
                            &Value::Null,
                            &body,
                            &actor_str,
                            now,
                        )
                        .await?;
                    match result {
                        Ok(_) => Ok(()),
                        Err(conflict) => Err(AppError::Internal(format!(
                            "unexpected version conflict: stored={}",
                            conflict.stored_version
                        ))),
                    }
                })
            })
            .await
    }
}
