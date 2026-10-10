//! The currencies the kiosk offers: the owner's manage and configure.
//! Methods of `KioskService` (design §10 S0); the type lives in `service.rs`.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::kiosk::commands::configure_currency::ConfigureCurrency;
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

    /// `configure`: switches a currency against the current publication.
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

        let actor = username(actor).to_owned();
        let at = cut_to_millis(now);
        let line = AuditLine {
            actor,
            subject: code.as_str().to_owned(),
            what: KioskAudit::CurrencyConfigured {
                enabled: cmd.enabled,
                buy: String::new(),
                sell: String::new(),
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
                    if enabled {
                        let has_both = repo.published_both_sides_in(&mut *tx, &b, code).await?;
                        if !has_both {
                            return Err(refuse(KioskServiceError::RatesFirst));
                        }
                    }
                    repo.set_switch_in(tx, &b, code, enabled).await?;
                    ports.record_in(tx, &b, &line, at).await
                })
            })
            .await
    }
}
