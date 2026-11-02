//! Quotes command (TX-3, §4.2, §6).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::domain::posting::{CashPlace, CashPostings};
use xc_cash::infrastructure::drawer_low_levels::DrawerLowLevels;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{
    CurrencyCode, CurrencyUnits, ForeignAmount, MinorUnit, Money, Side, UnitSize,
};

use xc_rate::domain::currency::denomination_groups::GroupCode;

use crate::application::workflows::exchange::drawer_check;
use crate::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use crate::application::workflows::save_step::SaveActor;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::quote::{DrawerShortfallDto, QuoteDto};
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, Locks, role_not_allowed_here,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::negative_stock::shortfall_message;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "quotes.ts")]
pub struct QuotesCommandInputLine {
    pub currency: String,
    pub shop_side: String,
    pub quantity: String,
    pub fee_thb: Option<String>,
    #[serde(default)]
    pub denomination_group: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "quotes.ts")]
pub struct QuotesCommandInput {
    pub branch: Option<String>,
    pub branch_id: Option<String>,
    pub customer_id: Option<String>,
    pub channel: Option<String>,
    #[serde(default)]
    pub lines: Vec<QuotesCommandInputLine>,
    #[serde(default)]
    #[ts(optional)]
    pub coupon_code: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub payment_method: Option<String>,
}

pub struct QuotesCommand {
    pool: sqlx::PgPool,
    service: Arc<QuoteService>,
}

impl QuotesCommand {
    pub fn new(pool: sqlx::PgPool, service: Arc<QuoteService>) -> Self {
        Self { pool, service }
    }
}

impl CommandAction for QuotesCommand {
    const NAME: &'static str = "quoteCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "saveTransaction");
    type Input = QuotesCommandInput;
    type Output = QuoteDto;
    const ROUTE: &'static str = "/api/quotes";
    const METHOD: Method = Method::POST;

    fn locks(&self, branch: &BranchId, input: &Self::Input) -> Locks {
        if let Some(ref b) = input.branch {
            Locks::One(BranchId::new(b))
        } else {
            Locks::One(branch.clone())
        }
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let channel = match input.channel.as_deref().unwrap_or("DESK") {
            "DESK" => QuoteChannel::Desk,
            "KIOSK" => QuoteChannel::Kiosk,
            "BOOKING" => QuoteChannel::Booking,
            "APP" => QuoteChannel::App,
            _ => {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("channel"),
                    "invalid channel",
                    StatusCode::BAD_REQUEST,
                ));
            }
        };

        if input.lines.len() != 1 {
            return Err(V3Error::new(
                "validationFailed",
                Some("lines"),
                "exactly one quote line required",
                StatusCode::BAD_REQUEST,
            ));
        }

        let raw_line = &input.lines[0];

        let curr = CurrencyCode::parse(&raw_line.currency).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("currency"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;

        let side = match raw_line.shop_side.as_str() {
            "BUY" => Side::Buy,
            "SELL" => Side::Sell,
            _ => {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("shopSide"),
                    "invalid shop side",
                    StatusCode::BAD_REQUEST,
                ));
            }
        };

        let (unit_size_dec, minor_unit_i16): (Decimal, i16) =
            sqlx::query_as("SELECT unit_size, minor_unit FROM currencies WHERE code = $1")
                .bind(curr.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| {
                    V3Error::new(
                        "serverError",
                        None,
                        e.to_string(),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                })?
                .ok_or_else(|| {
                    V3Error::new(
                        "validationFailed",
                        Some("currency"),
                        "unknown currency",
                        StatusCode::BAD_REQUEST,
                    )
                })?;

        let unit_size = UnitSize::from_decimal(unit_size_dec).map_err(|e| {
            V3Error::new(
                "serverError",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        let minor_unit = MinorUnit::new(minor_unit_i16 as u8).map_err(|e| {
            V3Error::new(
                "serverError",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        let units = CurrencyUnits::new(curr, unit_size, minor_unit);

        let quantity = ForeignAmount::parse(&raw_line.quantity, &units).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("quantity"),
                "ข้อมูลไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let fee_thb = match raw_line.fee_thb.as_deref() {
            Some(s) if !s.trim().is_empty() => {
                let m = Money::parse(s).map_err(|_| {
                    V3Error::new(
                        "validationFailed",
                        Some("feeThb"),
                        "invalid fee",
                        StatusCode::BAD_REQUEST,
                    )
                })?;
                Some(m)
            }
            _ => None,
        };

        let denomination_group = match raw_line.denomination_group.as_deref() {
            Some(s) if !s.trim().is_empty() => GroupCode::new(s).map_err(|_| {
                V3Error::new(
                    "validationFailed",
                    Some("denominationGroup"),
                    "invalid denomination group",
                    StatusCode::BAD_REQUEST,
                )
            })?,
            _ => GroupCode::all(),
        };

        let payment_method = match input.payment_method.as_deref() {
            Some(m) => Some(PaymentMethod::parse(m).ok_or_else(|| {
                V3Error::new(
                    "validationFailed",
                    Some("paymentMethod"),
                    "invalid payment method",
                    StatusCode::BAD_REQUEST,
                )
            })?),
            None => None,
        };

        let group_code = denomination_group.as_str().to_string();

        let req = QuoteRequest {
            branch_id: branch.clone(),
            customer_id: input.customer_id,
            channel,
            lines: vec![QuoteRequestLine {
                currency: curr,
                shop_side: side,
                quantity,
                fee_thb,
                denomination_group,
            }],
            payment_method,
        };

        let save_actor = SaveActor::from_actor(who).ok_or_else(role_not_allowed_here)?;

        let quote = match self
            .service
            .quote_with_coupon_in(
                tx,
                req,
                &save_actor,
                Utc::now(),
                input.coupon_code.as_deref(),
            )
            .await
        {
            Ok(q) => q,
            Err(pos_error::AppError::Domain(msg)) => {
                if msg.starts_with("rateUnavailable") {
                    return Err(V3Error::new(
                        "rateUnavailable",
                        None,
                        msg,
                        StatusCode::CONFLICT,
                    ));
                } else if msg.starts_with("couponInvalid") {
                    tracing::info!(reason = %msg, "coupon invalid on quote");
                    return Err(V3Error::new(
                        "couponInvalid",
                        None,
                        "couponInvalid",
                        StatusCode::BAD_REQUEST,
                    ));
                } else if msg.starts_with("outOfScope") {
                    return Err(V3Error::new("outOfScope", None, msg, StatusCode::FORBIDDEN));
                } else if msg.starts_with("validationFailed: customerId") {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("customerId"),
                        msg,
                        StatusCode::BAD_REQUEST,
                    ));
                } else if msg.starts_with("validationFailed:quantity:") {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("quantity"),
                        msg,
                        StatusCode::BAD_REQUEST,
                    ));
                } else {
                    return Err(V3Error::new(
                        "validationFailed",
                        None,
                        msg,
                        StatusCode::BAD_REQUEST,
                    ));
                }
            }
            Err(e) => {
                return Err(V3Error::new(
                    "serverError",
                    None,
                    e.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ));
            }
        };

        let pub_version: i32 =
            sqlx::query_scalar("SELECT version FROM rate_publications WHERE id = $1")
                .bind(quote.publication_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(|e| {
                    V3Error::new(
                        "serverError",
                        None,
                        e.to_string(),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                })?;

        let quote_needs: Vec<(CurrencyCode, Decimal)> = quote
            .lines
            .iter()
            .flat_map(|l| {
                let settle = Decimal::from_i128_with_scale(l.settle_thb.minor_i128(), 2);
                drawer_check::needs(
                    l.shop_side,
                    l.currency,
                    l.quantity.value(),
                    settle,
                    quote.cash_settlement,
                )
            })
            .collect();

        let mut dto = QuoteDto::from_domain(quote, pub_version).map_err(|e| {
            V3Error::new(
                "serverError",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;
        if channel == QuoteChannel::Desk {
            let shift_id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM shifts \
                 WHERE branch_id = $1 AND teller_id::text = $2 AND status = 'OPEN' \
                 ORDER BY opened_at DESC LIMIT 1",
            )
            .bind(branch.as_str())
            .bind(&who.user_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| {
                V3Error::new(
                    "serverError",
                    None,
                    e.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            })?;
            if let Some(shift_id) = shift_id {
                let shortfall = drawer_check::drawer_shortfall_in(tx, &shift_id, &quote_needs)
                    .await
                    .map_err(|e| {
                        V3Error::new(
                            "serverError",
                            None,
                            e.to_string(),
                            StatusCode::INTERNAL_SERVER_ERROR,
                        )
                    })?;
                if !shortfall.lines.is_empty() {
                    dto.drawer_shortfall_message = Some(shortfall_message(&shortfall));
                    dto.drawer_shortfall = shortfall
                        .lines
                        .into_iter()
                        .map(|l| DrawerShortfallDto {
                            currency: l.currency.as_str().to_owned(),
                            place: l.place.as_str().to_owned(),
                            short_by: l.short_by.normalize().to_string(),
                        })
                        .collect();
                } else if !quote_needs.is_empty() {
                    let levels = DrawerLowLevels::for_branch_in(tx, branch)
                        .await
                        .map_err(|e| {
                            V3Error::new(
                                "serverError",
                                None,
                                e.to_string(),
                                StatusCode::INTERNAL_SERVER_ERROR,
                            )
                        })?;
                    if !levels.is_empty() {
                        let balances = LedgerCashPostings::new()
                            .expected_in(tx, &CashPlace::Drawer(shift_id))
                            .await
                            .map_err(|e| {
                                V3Error::new(
                                    "serverError",
                                    None,
                                    e.to_string(),
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                )
                            })?;
                        let mut totals: std::collections::BTreeMap<&str, (CurrencyCode, Decimal)> =
                            std::collections::BTreeMap::new();
                        for &(currency, amount) in &quote_needs {
                            if amount > Decimal::ZERO {
                                totals
                                    .entry(currency.as_str())
                                    .and_modify(|(_, sum)| *sum += amount)
                                    .or_insert((currency, amount));
                            }
                        }
                        for (_, (currency, need)) in totals {
                            if let Some((_, level)) = levels.iter().find(|(c, _)| *c == currency) {
                                let have = balances.get(currency).value();
                                if have >= need && have - need <= *level {
                                    dto.below_low_cash_level.push(currency.as_str().to_owned());
                                }
                            }
                        }
                    }
                }
            }
        }
        let published = crate::presentation::http::v3::published_rates::published_rates_data(
            &self.pool, branch,
        )
        .await
        .map_err(|e| {
            V3Error::new(
                "serverError",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;
        for line in &mut dto.lines {
            if let Some(pub_line) = published
                .lines
                .iter()
                .find(|l| l.currency == line.currency && l.group == group_code)
            {
                line.pricing_hidden = pub_line.hidden;
                line.pricing_hidden_reason = pub_line.hidden_reason.clone();
            }
        }

        Ok(dto)
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let quote_service = Arc::new(QuoteService::new(state.db.clone()));
    CommandEndpoint::register(
        cfg,
        Arc::new(QuotesCommand::new(state.db.clone(), quote_service)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    QuotesCommandInput::export_all(&cfg)?;
    QuotesCommandInputLine::export_all(&cfg)?;
    QuoteDto::export_all(&cfg)?;
    Ok(())
}
