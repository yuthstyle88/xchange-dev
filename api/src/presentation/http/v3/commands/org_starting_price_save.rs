//! SP-1 (#1585, rates design §12.6, §12.7, §12.10): `POST /api/v3/commands/org_starting_price_save`
//!
//! Enters central organisation-level starting prices, mirrors them to rate references,
//! and prices unpublished branches that have an active policy.

use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::draft::reference_base::ReferenceBase;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};

use super::commands::{make_approval_service, map_rate_engine_error};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::org_starting_prices::{
    OrgStartingPriceSaveInput, OrgStartingPriceSaveOutput, UnpricedBranch,
};
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, role_not_allowed_here, server_error};
use crate::presentation::http::v3::error::V3Error;

pub struct OrgStartingPriceSaveCommand {
    draft_service: Arc<RateDraftService>,
    publish_service: Arc<PublishService>,
}

impl OrgStartingPriceSaveCommand {
    pub fn new(draft_service: Arc<RateDraftService>, publish_service: Arc<PublishService>) -> Self {
        Self {
            draft_service,
            publish_service,
        }
    }
}

fn validation_failed(field: &'static str, msg: &'static str) -> V3Error {
    V3Error::new("validationFailed", Some(field), msg, StatusCode::BAD_REQUEST)
}

impl CommandAction for OrgStartingPriceSaveCommand {
    const NAME: &'static str = "orgStartingPriceSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "orgStartingPriceSave");
    type Input = OrgStartingPriceSaveInput;
    type Output = OrgStartingPriceSaveOutput;
    const ROUTE: &'static str = "/api/v3/commands/org_starting_price_save";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let org_id = Uuid::parse_str(&who.organization_id).map_err(|_| role_not_allowed_here())?;

        if input.prices.is_empty() {
            return Err(validation_failed("prices", "prices cannot be empty"));
        }
        if input.reason.trim().is_empty() {
            return Err(validation_failed("reason", "reason cannot be empty"));
        }

        // Validate all price entries before writing anything
        struct ValidatedEntry {
            currency: String,
            buy: Option<Decimal>,
            sell: Option<Decimal>,
            unit_size: Decimal,
            raw: serde_json::Value,
        }

        let mut seen_currencies = HashSet::with_capacity(input.prices.len());
        let mut validated = Vec::with_capacity(input.prices.len());
        for p in &input.prices {
            let currency = p.currency.trim().to_uppercase();
            if currency.len() != 3 {
                return Err(validation_failed("currency", "invalid currency code"));
            }
            if !seen_currencies.insert(currency.clone()) {
                return Err(validation_failed(
                    "prices",
                    "currency appears more than once",
                ));
            }

            let buy = match &p.buy {
                Some(s) => match Decimal::from_str(s.trim()) {
                    Ok(d) => Some(d),
                    Err(_) => return Err(validation_failed("buy", "invalid buy rate")),
                },
                None => None,
            };

            let sell = match &p.sell {
                Some(s) => match Decimal::from_str(s.trim()) {
                    Ok(d) => Some(d),
                    Err(_) => return Err(validation_failed("sell", "invalid sell rate")),
                },
                None => None,
            };

            if buy.is_none() && sell.is_none() {
                return Err(validation_failed("prices", "buy and sell cannot both be null"));
            }
            if buy.is_some_and(|b| b <= Decimal::ZERO) {
                return Err(validation_failed("buy", "buy rate must be positive"));
            }
            if sell.is_some_and(|s| s <= Decimal::ZERO) {
                return Err(validation_failed("sell", "sell rate must be positive"));
            }
            if let (Some(b), Some(s)) = (buy, sell)
                && b > s
            {
                return Err(validation_failed(
                    "prices",
                    "buy rate cannot exceed sell rate",
                ));
            }

            let unit_size_row: Option<(Decimal,)> = sqlx::query_as(
                "SELECT unit_size FROM currencies WHERE code = $1",
            )
            .bind(&currency)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| server_error())?;

            let unit_size = match unit_size_row {
                Some((u,)) => u,
                None => return Err(validation_failed("currency", "unknown currency code")),
            };

            let raw = serde_json::to_value(p).map_err(|_| server_error())?;

            validated.push(ValidatedEntry {
                currency,
                buy,
                sell,
                unit_size,
                raw,
            });
        }

        let now = Utc::now();
        let source_date = now.date_naive();
        let provider = format!("START:{}", org_id);

        let raw_checksum = {
            let bytes = serde_json::to_vec(&input.prices).map_err(|_| server_error())?;
            format!("{:x}", Sha256::digest(&bytes))
        };

        let batch_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
             VALUES ($1, $2, $3, $4, $5, 'COMPLETE')",
        )
        .bind(batch_id)
        .bind(&provider)
        .bind(source_date)
        .bind(now)
        .bind(&raw_checksum)
        .execute(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        for entry in validated {
            let line_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
                 VALUES ($1, $2, $3, 'START', $4, $5, $6, $7)",
            )
            .bind(line_id)
            .bind(batch_id)
            .bind(&entry.currency)
            .bind(entry.unit_size)
            .bind(entry.buy)
            .bind(entry.sell)
            .bind(&entry.raw)
            .execute(&mut **tx)
            .await
            .map_err(|_| server_error())?;

            let price_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO org_starting_prices ( \
                     id, organization_id, currency_code, version, unit_size, buy, sell, entered_by, entered_at, reason, reference_line_id \
                 ) VALUES ( \
                     $1, $2, $3, \
                     COALESCE((SELECT MAX(version) FROM org_starting_prices WHERE organization_id = $2 AND currency_code = $3), 0) + 1, \
                     $4, $5, $6, $7, $8, $9, $10 \
                 )",
            )
            .bind(price_id)
            .bind(org_id)
            .bind(&entry.currency)
            .bind(entry.unit_size)
            .bind(entry.buy)
            .bind(entry.sell)
            .bind(&who.user_id)
            .bind(now)
            .bind(&input.reason)
            .bind(line_id)
            .execute(&mut **tx)
            .await
            .map_err(|_| server_error())?;
        }

        // §12.7: Find every branch of the organisation with current_publication_id IS NULL
        let unpublished: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM branches WHERE organization_id = $1 AND current_publication_id IS NULL ORDER BY id",
        )
        .bind(org_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        let all_currencies: Vec<(String,)> =
            sqlx::query_as("SELECT code FROM currencies ORDER BY code")
                .fetch_all(&mut **tx)
                .await
                .map_err(|_| server_error())?;

        let mut unpriced = Vec::new();
        for (b_id,) in unpublished {
            let b = BranchId::new(&b_id);
            let active_policy: Option<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM rate_policy_versions \
                 WHERE branch_id = $1 AND valid_from <= now() AND (valid_to IS NULL OR valid_to > now()) \
                 ORDER BY version DESC LIMIT 1",
            )
            .bind(b.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| server_error())?;

            if active_policy.is_none() {
                unpriced.push(UnpricedBranch {
                    branch: b_id,
                    reason: "noPolicy".to_string(),
                });
                continue;
            }

            let mut lines = Vec::new();
            for (code_str,) in &all_currencies {
                if let Ok(c) = CurrencyCode::parse(code_str.trim()) {
                    let ref_line = ReferenceBase::newest_in(tx, &b, &c)
                        .await
                        .map_err(map_rate_engine_error)?;
                    if ref_line.is_some() {
                        lines.push(DraftLine {
                            currency: c,
                            buy: LineSide::Auto,
                            sell: LineSide::Auto,
                            reason: None,
                        });
                    }
                }
            }

            if !lines.is_empty() {
                let draft = self
                    .draft_service
                    .create(tx, &b, who, lines)
                    .await
                    .map_err(map_rate_engine_error)?;
                let submit_key = format!("sp_submit_{}_{}", b.as_str(), draft.id);
                self.draft_service
                    .submit(tx, &b, who, &draft.id, &submit_key)
                    .await
                    .map_err(map_rate_engine_error)?;
                self.publish_service
                    .publish(tx, &b, who, &draft.id, None)
                    .await
                    .map_err(map_rate_engine_error)?;
            }
        }

        Ok(OrgStartingPriceSaveOutput { unpriced })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let approval_service = make_approval_service(state.db.clone());
    let draft_service = Arc::new(RateDraftService::new(state.db.clone(), approval_service));
    let publish_service = Arc::new(PublishService::new(state.db.clone()));
    CommandEndpoint::register(
        cfg,
        Arc::new(OrgStartingPriceSaveCommand::new(draft_service, publish_service)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::org_starting_prices::export_command_types(dir)
}
