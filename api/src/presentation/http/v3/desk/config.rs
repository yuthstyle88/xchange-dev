//! Desk configuration core reader (§4.1).

use std::collections::HashMap;
use std::sync::Arc;

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::UnitSize;
use xc_rate::application::branch::service::BranchService;
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

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

        let shift_repo = Arc::new(PgShiftRepository::new(state.db.clone()));
        let shift_service = ShiftService::new(shift_repo, branch_tx.clone());
        let shift_open = !shift_service.open_of_branch(branch).await?.is_empty();

        let admin_url = format!("/?branch={}", uri_component(&branch_id));

        let source: Arc<dyn RateSource> = Arc::new(BotRateSource::bot());
        let rate_service = RateService::new(branch_tx.clone(), source);
        let raw_rates = rate_service.current(branch).await?;

        let unit_size_rows =
            sqlx::query_as::<_, (String, Decimal)>("SELECT code, unit_size FROM currencies")
                .fetch_all(&state.db)
                .await
                .map_err(AppError::Database)?;
        let mut unit_sizes: HashMap<String, UnitSize> =
            HashMap::with_capacity(unit_size_rows.len());
        for (code, dec) in unit_size_rows {
            let u = UnitSize::from_decimal(dec)
                .map_err(|e| AppError::Internal(format!("invalid unit_size for {code}: {e}")))?;
            unit_sizes.insert(code.trim().to_string(), u);
        }

        let rates: Vec<DeskRate> = raw_rates
            .into_iter()
            .filter(|r| !r.code.is_thb())
            .map(|r| {
                let code_str = r.code.as_str().to_string();
                let unit_size = unit_sizes.get(&code_str).ok_or_else(|| {
                    AppError::Internal(format!("missing currency unit_size for {code_str}"))
                })?;
                let publication = match (r.buy.is_some() || r.sell.is_some(), r.updated_at) {
                    (true, Some(t)) => Some(t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
                    (true, None) => {
                        return Err(AppError::Internal(format!(
                            "rate {} has no publication timestamp",
                            r.code
                        )));
                    }
                    (false, Some(t)) => {
                        Some(t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                    }
                    (false, None) => None,
                };
                Ok(DeskRate {
                    code: code_str,
                    unit_size: unit_size.to_string(),
                    buy: r.buy.as_ref().map(|b| b.to_string()),
                    sell: r.sell.as_ref().map(|s| s.to_string()),
                    publication,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;

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
