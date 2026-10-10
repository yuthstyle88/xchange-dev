//! QuoteService workflow (§4.3, §7).

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel, QuoteStatus};
use xc_exchange::domain::quote::quote_line::QuoteLine;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::settings::repository::SettingsRepository;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::settings::repository::PgSettingsRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ExchangeArithmetic, ForeignAmount, Money, Side};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::publication::published_rates::{LineSelector, PublishedRates};
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

use crate::application::workflows::save_step::SaveActor;

#[derive(Debug, Clone)]
pub struct QuoteRequestLine {
    pub currency: CurrencyCode,
    pub shop_side: Side,
    pub quantity: ForeignAmount,
    pub fee_thb: Option<Money>,
    pub denomination_group: GroupCode,
}

#[derive(Debug, Clone)]
pub struct QuoteRequest {
    pub branch_id: BranchId,
    pub customer_id: Option<String>,
    pub channel: QuoteChannel,
    pub lines: Vec<QuoteRequestLine>,
}

pub struct QuoteService {
    #[allow(dead_code)]
    pool: PgPool,
    published_rates: PgPublishedRates,
    quote_repo: PgQuoteRepository,
    settings_repo: PgSettingsRepository,
}

impl QuoteService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            published_rates: PgPublishedRates::new(pool.clone()),
            quote_repo: PgQuoteRepository::new(pool.clone()),
            settings_repo: PgSettingsRepository::new(pool.clone()),
            pool,
        }
    }

    pub async fn quote_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        req: QuoteRequest,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<Quote> {
        // Step 1: Exactly 1 line
        if req.lines.len() != 1 {
            return Err(AppError::Domain(
                "validationFailed: exactly one line required".to_string(),
            ));
        }
        let line_req = &req.lines[0];

        // Step 2: Validate quantity
        let qty_dec = line_req.quantity.value();
        if qty_dec <= Decimal::ZERO {
            return Err(AppError::Domain(
                "validationFailed: quantity must be greater than zero".to_string(),
            ));
        }

        let minor_unit: Option<i16> =
            sqlx::query_scalar("SELECT minor_unit FROM currencies WHERE code = $1")
                .bind(line_req.currency.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;

        let minor_unit = match minor_unit {
            Some(mu) => mu as u32,
            None => {
                return Err(AppError::Domain(
                    "rateUnavailable: currency not found".to_string(),
                ));
            }
        };

        if qty_dec.normalize().scale() > minor_unit {
            return Err(AppError::Domain(
                "validationFailed: quantity does not fit minor unit".to_string(),
            ));
        }

        // Step 3: Fetch published rate line
        let published_side = self
            .published_rates
            .line_for(
                tx,
                &req.branch_id,
                LineSelector {
                    code: line_req.currency,
                    side: line_req.shop_side,
                    group: line_req.denomination_group.clone(),
                    quantity: Some(line_req.quantity),
                },
            )
            .await?;

        let published_side = match published_side {
            Some(s) => s,
            None => {
                return Err(AppError::Domain(
                    "rateUnavailable: no published line".to_string(),
                ));
            }
        };

        // Step 4: Arithmetic
        let thb_amount = ExchangeArithmetic::thb(
            line_req.quantity,
            published_side.rate,
            published_side.unit_size,
        );

        let fee_thb = line_req.fee_thb.unwrap_or(Money::ZERO);
        if fee_thb > thb_amount {
            return Err(AppError::Domain(
                "validationFailed: fee exceeds thb amount".to_string(),
            ));
        }

        let settle_thb = match line_req.shop_side {
            Side::Buy => {
                let settle_minor = thb_amount.minor() - fee_thb.minor();
                Money::from_minor(settle_minor)
                    .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?
            }
            Side::Sell => {
                let settle_minor = thb_amount.minor() + fee_thb.minor();
                Money::from_minor(settle_minor)
                    .map_err(|_| AppError::Domain("invalid settle amount".to_string()))?
            }
        };

        // Step 5: Staleness check under policy version the line was published under
        if published_side.stale {
            let stale_policy = self
                .published_rates
                .stale_policy_in(tx, &published_side)
                .await?;
            stale_policy
                .admits(&published_side, thb_amount)
                .map_err(|_| AppError::Domain("rateUnavailable: stale line refused".to_string()))?;
        }

        // Step 6: Organization & TTL
        let org_id: Option<Uuid> =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                .bind(req.branch_id.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;

        let org_id = match org_id {
            Some(id) => id,
            None => {
                return Err(AppError::Domain(
                    "validationFailed: branch not found".to_string(),
                ));
            }
        };

        let ttl_seconds = self.settings_repo.get_ttl_in(tx, org_id).await?;
        let expires_at = now + Duration::seconds(ttl_seconds as i64);

        let quote_id = Uuid::new_v4();
        let line_id = Uuid::new_v4();

        let quote_line = QuoteLine {
            id: line_id,
            organization_id: org_id,
            quote_id,
            published_rate_line_id: published_side.line_id.as_uuid(),
            shop_side: line_req.shop_side,
            currency: line_req.currency,
            denomination_group: line_req.denomination_group.to_string(),
            quantity: line_req.quantity,
            unit_size: published_side.unit_size,
            base_rate: published_side.rate,
            final_rate: published_side.rate,
            thb_amount,
            fee_thb,
            settle_thb,
            line_no: 1,
            created_at: now,
            updated_at: now,
        };

        let quote = Quote {
            id: quote_id,
            organization_id: org_id,
            branch_id: req.branch_id,
            customer_id: req.customer_id,
            quote_type: "INDICATIVE".to_string(),
            channel: req.channel,
            status: QuoteStatus::Active,
            publication_id: published_side.publication.id.as_uuid(),
            expires_at,
            benefit_snapshot: serde_json::json!({}),
            created_by: who.id.clone(),
            accepted_by_transaction_id: None,
            created_at: now,
            updated_at: now,
            lines: vec![quote_line],
            stale: published_side.stale,
        };

        // Step 7: Write to DB last
        self.quote_repo.insert_in(tx, &quote).await?;

        Ok(quote)
    }
}
