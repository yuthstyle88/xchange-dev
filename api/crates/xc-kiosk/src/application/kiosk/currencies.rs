//! The currencies the kiosk offers: the owner's manage and configure.
//! Methods of `KioskService` (design §10 S0); the type lives in `service.rs`.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, plain_decimal};

use crate::domain::kiosk::commands::configure_currency::ConfigureCurrency;
use crate::domain::kiosk::commands::save_counter_rate::SaveCounterRate;
use crate::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use crate::domain::kiosk::value_objects::audit_line::{AuditLine, KioskAudit};
use crate::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_currencies::KioskCurrencies;
use crate::domain::kiosk::value_objects::kiosk_service_error::KioskServiceError;

use super::service::{KioskService, cut_to_millis, refuse, username};

impl KioskService {
    /// `manage`: every currency with whether this branch's kiosk offers it.
    pub async fn manage(
        &self,
        branch: &BranchId,
        _actor: &KioskActor,
    ) -> AppResult<Vec<CurrencySwitch>> {
        self.branch_name(branch).await?;
        Ok(KioskCurrencies::new(&self.repo.switches(branch).await?).all())
    }

    /// `configure`: saves the counter rate, then stores the switch.
    pub async fn configure(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        cmd: ConfigureCurrency,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.branch_name(branch).await?;
        let code = CurrencyCode::parse(&cmd.code)
            .ok()
            .filter(|code| !code.is_thb())
            .ok_or_else(|| refuse(KioskServiceError::InvalidCurrencySettings))?;
        let rates_valid = match (plain_decimal(&cmd.buy), plain_decimal(&cmd.sell)) {
            (Some(buy), Some(sell)) => buy > Decimal::ZERO && sell >= buy,
            _ => false,
        };
        if !rates_valid {
            return Err(refuse(KioskServiceError::RatesFirst));
        }

        let actor = username(actor).to_owned();
        let rate = SaveCounterRate {
            code,
            buy: cmd.buy.clone(),
            sell: cmd.sell.clone(),
            previous_updated: cmd.previous_updated,
            partner_name: cmd.partner_name,
            partner_cost: cmd.partner_cost,
            request_id: cmd.request_id,
        };
        self.ports.save_rate(branch, &rate, &actor, now).await?;

        let at = cut_to_millis(now);
        let line = AuditLine {
            actor,
            subject: code.as_str().to_owned(),
            what: KioskAudit::CurrencyConfigured {
                enabled: cmd.enabled,
                buy: cmd.buy,
                sell: cmd.sell,
            },
        };
        let (repo, ports, b, enabled) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            cmd.enabled,
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    repo.set_switch_in(tx, &b, code, enabled).await?;
                    ports.record_in(tx, &b, &line, at).await
                })
            })
            .await
    }
}
