//! Desk configuration core reader (§4.1).

use std::sync::Arc;

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xc_rate::domain::publication::published_rates::PublishedRates;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;
use xc_shift::application::shift::service::ShiftService;

use crate::application::settings::payment_settings::PaymentSettingsService;
use crate::bootstrap::AppState;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::presentation::dto::v3::desk_config::{DeskConfig, DeskPaymentMethod, DeskRate};

fn uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        let kept = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            );
        if kept {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub struct DeskConfigCore;

impl DeskConfigCore {
    /// Reads desk configuration for `branch` (§4.1).
    pub async fn read(state: &AppState, branch: &BranchId) -> AppResult<DeskConfig> {
        let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));

        let branch_service = BranchService::new(branch_tx.clone());
        let here = branch_service.require(branch).await?;
        let branch_id = here.id.as_str().to_string();
        let branch_name = here.name.as_str().to_string();

        let shift_repo = Arc::new(
            xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                state.db.clone(),
            ),
        );
        let shift_service = ShiftService::new(shift_repo, branch_tx.clone());
        let shift_open = !shift_service.open_of_branch(branch).await?.is_empty();

        let admin_url = format!("/?branch={}", uri_component(&branch_id));

        let mut tx = state.db.begin().await.map_err(AppError::Database)?;
        let published_rates = PgPublishedRates::new(state.db.clone());
        let sellable = published_rates.sellable(&mut tx, branch).await?;

        let rates: Vec<DeskRate> = sellable
            .into_iter()
            .map(|r| DeskRate {
                code: r.code.as_str().to_string(),
                unit_size: r.unit_size.to_string(),
                buy: r.buy.map(|b| b.to_string()),
                sell: r.sell.map(|s| s.to_string()),
                publication: Some(
                    r.published_at
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                ),
            })
            .collect();

        let store = BranchSettingStore::new(state.db.clone());
        let branch_repo = PgBranchRepository::new(state.db.clone());
        let payment_service = PaymentSettingsService::new(branch_tx, store, branch_repo);
        let rules = payment_service.rules(branch).await?;
        let payment_methods = rules
            .rows()
            .iter()
            .map(|r| DeskPaymentMethod {
                id: r.id.clone(),
                enabled: r.enabled,
                buy: r.buy,
                sell: r.sell,
                account: r.account.clone(),
                instruction: r.instruction.clone(),
            })
            .collect();

        Ok(DeskConfig {
            branch_id,
            branch_name,
            shift_open,
            admin_url,
            rates,
            payment_methods,
        })
    }
}
