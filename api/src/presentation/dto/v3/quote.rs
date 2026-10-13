//! Quote DTOs for v3 API (§4.2).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_exchange::domain::quote::quote::Quote;
use xc_kernel::money::Side;

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
    pub settle_thb: String,
    pub published_rate_line_id: String,
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
    #[ts(type = "Array<any>")]
    pub benefits: Vec<serde_json::Value>,
    pub stale: bool,
}

impl QuoteDto {
    pub fn from_domain(quote: Quote, pub_version: i32) -> Self {
        Self {
            id: quote.id.to_string(),
            quote_type: quote.quote_type,
            channel: quote.channel.as_str().to_string(),
            expires_at: quote.expires_at.to_rfc3339(),
            publication: QuotePublicationDto {
                id: quote.publication_id.to_string(),
                version: pub_version,
            },
            lines: quote
                .lines
                .into_iter()
                .map(|line| QuoteLineDto {
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
                    settle_thb: line.settle_thb.to_string(),
                    published_rate_line_id: line.published_rate_line_id.to_string(),
                })
                .collect(),
            benefits: quote
                .benefit_snapshot
                .get("applied")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
            stale: quote.stale,
        }
    }
}
