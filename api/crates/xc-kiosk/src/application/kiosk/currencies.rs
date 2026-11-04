//! The currencies the kiosk offers: the owner's manage and configure.
//! Methods of `KioskService` (design §10 S0); the type lives in `service.rs`.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::kiosk::commands::configure_currency::{ConfigureCurrency, ConfigureDisplay};
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

    /// `configure_display`: sets `branch_currencies.display` (`SHOW`, `ASK`, `HIDE`) for one row.
    pub async fn configure_display(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        cmd: ConfigureDisplay,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.branch_name(branch).await?;
        let code = CurrencyCode::parse(cmd.code.trim())
            .ok()
            .filter(|code| !code.is_thb())
            .ok_or_else(|| refuse(KioskServiceError::InvalidCurrencySettings))?;
        let group = if cmd.group.trim().is_empty() {
            "ALL".to_owned()
        } else {
            cmd.group.trim().to_owned()
        };
        if group.len() > 16
            || !group
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(refuse(KioskServiceError::InvalidCurrencySettings));
        }
        let display = cmd.display.trim().to_owned();
        if !matches!(display.as_str(), "SHOW" | "ASK" | "HIDE") {
            return Err(refuse(KioskServiceError::InvalidCurrencySettings));
        }

        let actor = username(actor).to_owned();
        let at = cut_to_millis(now);
        let (repo, ports, b) = (self.repo.clone(), self.ports.clone(), branch.clone());
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    if display == "SHOW" {
                        let has_both = repo.published_both_sides_in(&mut *tx, &b, code).await?;
                        if !has_both {
                            return Err(refuse(KioskServiceError::RatesFirst));
                        }
                    }
                    let prev: Option<String> = sqlx::query_scalar(
                        "SELECT display FROM branch_currencies \
                         WHERE branch_id = $1 AND currency_code = $2 AND group_code = $3",
                    )
                    .bind(b.as_str())
                    .bind(code.as_str())
                    .bind(&group)
                    .fetch_optional(&mut **tx)
                    .await?;
                    let from = prev.unwrap_or_else(|| "HIDE".to_owned());

                    repo.set_display_in(tx, &b, code, &group, &display, &actor)
                        .await?;

                    if display == "SHOW" || display == "ASK" {
                        sqlx::query(
                            "INSERT INTO branch_currency_notes (branch_id, currency_code, group_code, denomination_id) \
                             SELECT $1, d.currency_code, $3, d.id \
                             FROM denominations d \
                             JOIN branches br ON br.id = $1 AND br.organization_id = d.organization_id \
                             WHERE d.currency_code = $2 \
                               AND d.group_code = $3 \
                               AND d.face_value IS NOT NULL \
                               AND NOT EXISTS ( \
                                   SELECT 1 FROM branch_currency_notes n \
                                   WHERE n.branch_id = $1 AND n.currency_code = $2 AND n.denomination_id = d.id \
                               ) \
                             ON CONFLICT DO NOTHING",
                        )
                        .bind(b.as_str())
                        .bind(code.as_str())
                        .bind(&group)
                        .execute(&mut **tx)
                        .await?;

                        let has_notes: bool = sqlx::query_scalar(
                            "SELECT EXISTS( \
                                SELECT 1 FROM branch_currency_notes \
                                WHERE branch_id = $1 AND currency_code = $2 AND group_code = $3 \
                            )",
                        )
                        .bind(b.as_str())
                        .bind(code.as_str())
                        .bind(&group)
                        .fetch_one(&mut **tx)
                        .await?;
                        if !has_notes {
                            return Err(refuse(KioskServiceError::NotesFirst));
                        }
                    }

                    let line = AuditLine {
                        actor,
                        subject: code.as_str().to_owned(),
                        what: KioskAudit::CurrencyDisplaySet {
                            group,
                            display,
                            from,
                        },
                    };
                    ports.record_in(tx, &b, &line, at).await
                })
            })
            .await
    }
}
