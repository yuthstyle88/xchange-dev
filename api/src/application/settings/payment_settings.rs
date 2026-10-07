//! Payment settings application service (PM-API, design §5.5.2).

use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::payment::PaymentRules;
use xc_rate::domain::branch::repositories::branch_repository::BranchRepository;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;

use crate::application::settings::payment_json::{rules_from_json, rules_json};
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::infrastructure::settings::stored_setting::VersionConflict;
use crate::presentation::dto::payment_methods::{PaymentAuditEntry, PaymentMethodsView};

const SETTING_NAME: &str = "paymentMethods";

#[derive(Debug, thiserror::Error)]
pub enum PaymentSaveError {
    #[error("version conflict")]
    Conflict(VersionConflict),
    #[error(transparent)]
    Database(#[from] AppError),
}

#[derive(Clone)]
pub struct PaymentSettingsService {
    branch_tx: BranchTx,
    store: BranchSettingStore,
    branches: PgBranchRepository,
}

impl PaymentSettingsService {
    pub fn new(
        branch_tx: BranchTx,
        store: BranchSettingStore,
        branches: PgBranchRepository,
    ) -> Self {
        Self {
            branch_tx,
            store,
            branches,
        }
    }

    /// Checks whether the branch exists in the database.
    pub async fn branch_exists(&self, branch: &BranchId) -> AppResult<bool> {
        self.branches.branch_exists(branch).await
    }

    /// Reads payment rules for a branch (defaults if no row).
    pub async fn rules(&self, branch: &BranchId) -> AppResult<PaymentRules> {
        let stored = self.store.read(branch, SETTING_NAME).await?;
        match stored {
            Some(s) => Ok(rules_from_json(&s.body).unwrap_or_else(PaymentRules::defaults)),
            None => Ok(PaymentRules::defaults()),
        }
    }

    /// Reads payment rules for a branch inside an existing transaction (defaults if no row).
    pub async fn rules_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
    ) -> AppResult<PaymentRules> {
        let stored = self.store.read_in(tx, branch, SETTING_NAME).await?;
        match stored {
            Some(s) => Ok(rules_from_json(&s.body).unwrap_or_else(PaymentRules::defaults)),
            None => Ok(PaymentRules::defaults()),
        }
    }

    /// Returns the view of payment methods for a branch and actor.
    pub async fn view(&self, branch: &BranchId, actor: &Actor) -> AppResult<PaymentMethodsView> {
        let stored = self.store.read(branch, SETTING_NAME).await?;
        let can_edit = actor.kind() == ActorKind::Owner;
        let (version, methods, updated_at, updated_by) = match stored {
            Some(s) => {
                let rules = rules_from_json(&s.body).unwrap_or_else(PaymentRules::defaults);
                (
                    s.version,
                    rules_json(&rules, true),
                    Some(s.updated_at.to_rfc3339_opts(SecondsFormat::Millis, true)),
                    Some(s.updated_by),
                )
            }
            None => (0, rules_json(&PaymentRules::defaults(), true), None, None),
        };

        let history = if can_edit {
            let entries = self.store.history(branch, SETTING_NAME, 20).await?;
            entries
                .into_iter()
                .map(|e| PaymentAuditEntry {
                    version: e.version,
                    actor: e.actor,
                    created_at: e.at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    before_methods: e.before,
                    after_methods: e.after,
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(PaymentMethodsView {
            branch_id: branch.as_str().to_string(),
            version,
            methods,
            updated_at,
            updated_by,
            can_edit,
            history,
        })
    }

    /// Saves payment methods through BranchTx::run, enforcing version check and writing history.
    pub async fn save(
        &self,
        branch: &BranchId,
        expected_version: i64,
        rules: PaymentRules,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<PaymentMethodsView, PaymentSaveError> {
        let store = self.store.clone();
        let branch_clone = branch.clone();
        let actor_str = actor.to_string();
        let first_before = rules_json(&PaymentRules::defaults(), true);
        let body = rules_json(&rules, true);

        let stored_result = self
            .branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    store
                        .write_in(
                            tx,
                            &branch_clone,
                            SETTING_NAME,
                            expected_version,
                            &first_before,
                            &body,
                            &actor_str,
                            now,
                        )
                        .await
                })
            })
            .await?;

        let stored = match stored_result {
            Ok(s) => s,
            Err(conflict) => return Err(PaymentSaveError::Conflict(conflict)),
        };

        let entries = self.store.history(branch, SETTING_NAME, 20).await?;
        let history = entries
            .into_iter()
            .map(|e| PaymentAuditEntry {
                version: e.version,
                actor: e.actor,
                created_at: e.at.to_rfc3339_opts(SecondsFormat::Millis, true),
                before_methods: e.before,
                after_methods: e.after,
            })
            .collect();

        Ok(PaymentMethodsView {
            branch_id: branch.as_str().to_string(),
            version: stored.version,
            methods: stored.body,
            updated_at: Some(
                stored
                    .updated_at
                    .to_rfc3339_opts(SecondsFormat::Millis, true),
            ),
            updated_by: Some(stored.updated_by),
            can_edit: true,
            history,
        })
    }
}
