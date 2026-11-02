//! Quote DTOs for v3 API (§4.2).

use pos_error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_exchange::domain::quote::quote::Quote;
use xc_kernel::money::Side;
use xc_marketing::domain::benefit::rule::BenefitClass;
use xc_marketing::domain::engine::snapshot::AppliedBenefit;

use crate::application::workflows::exchange::quote::reserves_benefits;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export_to = "quote.ts")]
pub enum QuoteBenefitBasis {
    Indicative,
    Reserved,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "quote.ts")]
pub struct QuoteBenefitDto {
    pub rule_id: String,
    pub version_id: String,
    pub benefit_type: String,
    #[ts(type = "\"PRICE\" | \"REWARD\"")]
    pub benefit_class: String,
    pub coupon_id: Option<String>,
    pub reservation_id: Option<String>,
    pub amount_thb: String,
    pub rate_before: String,
    pub rate_after: String,
    pub fee_before: String,
    pub fee_after: String,
    pub floor_capped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "quote.ts")]
pub struct QuotePublicationDto {
    pub id: String,
    pub version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "quote.ts")]
pub struct QuoteLineDto {
    pub currency: String,
    pub shop_side: String,
    pub quantity: String,
    pub unit_size: String,
    pub base_rate: String,
    pub final_rate: String,
    pub thb_amount: String,
    pub fee_thb: String,
    pub rounding_thb: String,
    pub settle_thb: String,
    pub published_rate_line_id: String,
    /// Whether the board hides this line (`pricing-tiers.md` §15.7); the desk still quotes it and warns the teller.
    pub pricing_hidden: bool,
    /// `STALE` · `BLOCKED` · `MISMATCH`, or `null`.
    pub pricing_hidden_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "quote.ts")]
pub struct DrawerShortfallDto {
    pub currency: String,
    pub place: String,
    pub short_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "quote.ts")]
pub struct QuoteDto {
    pub id: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub quote_type: String,
    pub channel: String,
    pub expires_at: String,
    pub publication: QuotePublicationDto,
    pub lines: Vec<QuoteLineDto>,
    pub benefits: Vec<QuoteBenefitDto>,
    pub benefit_basis: QuoteBenefitBasis,
    pub stale: bool,
    pub cash_settlement: bool,
    pub drawer_shortfall: Vec<DrawerShortfallDto>,
    pub drawer_shortfall_message: Option<String>,
    pub below_low_cash_level: Vec<String>,
}

impl QuoteDto {
    /// The answer for `quote`. Fallible (§6.4.3, BE-5): a snapshot entry that does not decode is an
    /// `AppError::Internal` naming the quote (a 500 `serverError`), never a silently dropped row. The
    /// skeleton keeps today's body; BE-5 types `benefits` and adds `benefitBasis`.
    pub fn from_domain(quote: Quote, pub_version: i32) -> AppResult<Self> {
        let benefit_basis = if reserves_benefits(quote.channel) {
            QuoteBenefitBasis::Reserved
        } else {
            QuoteBenefitBasis::Indicative
        };

        let benefits = match quote
            .benefit_snapshot
            .get("applied")
            .and_then(|v| v.as_array())
        {
            Some(entries) => entries
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    let applied =
                        serde_json::from_value::<AppliedBenefit>(entry.clone()).map_err(|e| {
                            AppError::Internal(format!(
                                "quote {}: benefit_snapshot.applied[{i}] does not decode: {e}",
                                quote.id
                            ))
                        })?;
                    Ok(QuoteBenefitDto {
                        rule_id: applied.rule_id.to_string(),
                        version_id: applied.version_id.to_string(),
                        benefit_type: applied.benefit_type,
                        benefit_class: match applied.benefit_class {
                            BenefitClass::Price => "PRICE".to_string(),
                            BenefitClass::Reward => "REWARD".to_string(),
                        },
                        coupon_id: applied.coupon_id.map(|id| id.to_string()),
                        reservation_id: applied.reservation_id.map(|id| id.to_string()),
                        amount_thb: applied.amount_thb,
                        rate_before: applied.rate_before,
                        rate_after: applied.rate_after,
                        fee_before: applied.fee_before,
                        fee_after: applied.fee_after,
                        floor_capped: applied.floor_capped,
                    })
                })
                .collect::<AppResult<Vec<_>>>()?,
            None => Vec::new(),
        };

        Ok(Self {
            id: quote.id.to_string(),
            quote_type: quote.quote_type.as_str().to_string(),
            channel: quote.channel.as_str().to_string(),
            expires_at: quote.expires_at.to_rfc3339(),
            publication: QuotePublicationDto {
                id: quote.publication_id.to_string(),
                version: pub_version,
            },
            lines: quote
                .lines
                .into_iter()
                .map(|line| {
                    let rounding_thb = format!("{:.2}", line.rounding_thb());
                    QuoteLineDto {
                        currency: line.currency.to_string(),
                        shop_side: match line.shop_side {
                            Side::Buy => "BUY".to_string(),
                            Side::Sell => "SELL".to_string(),
                        },
                        quantity: line.quantity.to_string(),
                        unit_size: line.unit_size.to_string(),
                        base_rate: line.base_rate.to_string(),
                        final_rate: line.final_rate.to_string(),
                        thb_amount: line.thb_amount.to_string(),
                        fee_thb: line.fee_thb.to_string(),
                        rounding_thb,
                        settle_thb: line.settle_thb.to_string(),
                        published_rate_line_id: line.published_rate_line_id.to_string(),
                        pricing_hidden: false,
                        pricing_hidden_reason: None,
                    }
                })
                .collect(),
            benefits,
            benefit_basis,
            stale: quote.stale,
            cash_settlement: quote.cash_settlement,
            drawer_shortfall: Vec::new(),
            drawer_shortfall_message: None,
            below_low_cash_level: Vec::new(),
        })
    }
}
