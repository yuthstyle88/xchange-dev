//! BQ-5: command `bindingQuoteIssue` (design `docs/design/blueprint/binding-quote.md` D-12, D-13; Q1, Q6, Q13–Q15).
//!
//! `POST /api/quotes/binding`. Who may call: `BindingQuoteIssue` (policy area `xc-auth/.../areas/binding_quote.rs`).
//! The body is `quoteCreate`'s plus `approvalRequestId` and `replacesQuoteId`; `channel` defaults to `DESK`.
//! The work is `IssueBindingQuote::run_in`. Answers:
//! - issued ⇒ 200 `{ quote, reservedLines }`: `quote` is `quoteCreate`'s `QuoteDto` (its `type` is `BINDING`, its
//!   `expiresAt` the lock's end), `reservedLines` one per quote line (D-12);
//! - over the branch limit ⇒ 409 `approvalRequired`, message `approvalRequired: <request id>`;
//! - `validationFailed` with field `channel` / `customerId` / `quantity` / `replacesQuoteId` as the workflow's prefix
//!   says (else no field), `stockInsufficient` 409, `staleVersion` 409, `approvalRejected` 409, `rateUnavailable` 409,
//!   `couponInvalid` 400, `outOfScope` 403;
//! - a replay of the same `Idempotency-Key` answers the first answer and issues nothing (D-7, the seam).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{
    CurrencyCode, CurrencyUnits, ForeignAmount, MinorUnit, Money, Side, UnitSize,
};
use xc_rate::domain::currency::denomination_groups::GroupCode;

use crate::application::workflows::exchange::binding_issue::{
    BindingIssueOutcome, BindingIssueRequest, IssueBindingQuote,
};
use crate::application::workflows::exchange::quote::{QuoteRequest, QuoteRequestLine};
use crate::application::workflows::save_step::SaveActor;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::quote::QuoteDto;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, Locks, role_not_allowed_here,
};
use crate::presentation::http::v3::commands::make_approval_service;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::quotes::QuotesCommandInputLine;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "binding_quote_issue.ts")]
pub struct BindingQuoteIssueInput {
    pub branch: Option<String>,
    pub customer_id: Option<String>,
    /// `DESK` when absent; any other channel is refused (D-14).
    #[serde(default)]
    #[ts(optional)]
    pub channel: Option<String>,
    #[serde(default)]
    pub lines: Vec<QuotesCommandInputLine>,
    #[serde(default)]
    #[ts(optional)]
    pub coupon_code: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub payment_method: Option<String>,
    /// The `BINDING_QUOTE` request a retry names (D-15).
    #[serde(default)]
    #[ts(optional)]
    pub approval_request_id: Option<String>,
    /// The ACTIVE binding quote this one replaces (D-6).
    #[serde(default)]
    #[ts(optional)]
    pub replaces_quote_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "binding_quote_issue.ts")]
pub struct ReservedLineDto {
    pub quote_line_id: String,
    pub reserved_quantity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "binding_quote_issue.ts")]
pub struct BindingQuoteIssueOutput {
    pub quote: QuoteDto,
    pub reserved_lines: Vec<ReservedLineDto>,
}

pub struct BindingQuoteIssueCommand {
    service: Arc<IssueBindingQuote>,
}

impl BindingQuoteIssueCommand {
    pub fn new(service: Arc<IssueBindingQuote>) -> Self {
        Self { service }
    }
}

impl CommandAction for BindingQuoteIssueCommand {
    const NAME: &'static str = "bindingQuoteIssue";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "bindingQuoteIssue");
    type Input = BindingQuoteIssueInput;
    type Output = BindingQuoteIssueOutput;
    const ROUTE: &'static str = "/api/quotes/binding";
    const METHOD: Method = Method::POST;

    fn locks(&self, branch: &BranchId, input: &Self::Input) -> Locks {
        match input.branch {
            Some(ref b) => Locks::One(BranchId::new(b)),
            None => Locks::One(branch.clone()),
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

        let replaces = match input.replaces_quote_id.as_deref() {
            Some(s) => Some(Uuid::parse_str(s).map_err(|_| {
                V3Error::new(
                    "validationFailed",
                    Some("replacesQuoteId"),
                    "invalid replacesQuoteId",
                    StatusCode::BAD_REQUEST,
                )
            })?),
            None => None,
        };

        let req = BindingIssueRequest {
            quote: QuoteRequest {
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
            },
            coupon_code: input.coupon_code,
            approval_request_id: input.approval_request_id,
            replaces,
        };

        let actor = SaveActor::from_actor(who).ok_or_else(role_not_allowed_here)?;

        let issued = match self.service.run_in(tx, req, &actor, Utc::now()).await {
            Ok(BindingIssueOutcome::Issued(q)) => *q,
            Ok(BindingIssueOutcome::ApprovalRequired(id)) => {
                return Err(V3Error::new(
                    "approvalRequired",
                    None,
                    format!("approvalRequired: {id}"),
                    StatusCode::CONFLICT,
                ));
            }
            Err(pos_error::AppError::Domain(msg)) => {
                return Err(if msg.starts_with("validationFailed:channel:") {
                    V3Error::new(
                        "validationFailed",
                        Some("channel"),
                        msg,
                        StatusCode::BAD_REQUEST,
                    )
                } else if msg.starts_with("validationFailed: customerId") {
                    V3Error::new(
                        "validationFailed",
                        Some("customerId"),
                        msg,
                        StatusCode::BAD_REQUEST,
                    )
                } else if msg.starts_with("validationFailed:quantity:") {
                    V3Error::new(
                        "validationFailed",
                        Some("quantity"),
                        msg,
                        StatusCode::BAD_REQUEST,
                    )
                } else if msg.starts_with("validationFailed:replacesQuoteId:") {
                    V3Error::new(
                        "validationFailed",
                        Some("replacesQuoteId"),
                        msg,
                        StatusCode::BAD_REQUEST,
                    )
                } else if msg.starts_with("validationFailed") {
                    V3Error::new("validationFailed", None, msg, StatusCode::BAD_REQUEST)
                } else if msg.starts_with("stockInsufficient") {
                    V3Error::new("stockInsufficient", None, msg, StatusCode::CONFLICT)
                } else if msg.starts_with("staleVersion") {
                    V3Error::new("staleVersion", None, msg, StatusCode::CONFLICT)
                } else if msg.starts_with("approvalRejected") {
                    V3Error::new("approvalRejected", None, msg, StatusCode::CONFLICT)
                } else if msg.starts_with("rateUnavailable") {
                    V3Error::new("rateUnavailable", None, msg, StatusCode::CONFLICT)
                } else if msg.starts_with("couponInvalid") {
                    V3Error::new(
                        "couponInvalid",
                        None,
                        "couponInvalid",
                        StatusCode::BAD_REQUEST,
                    )
                } else if msg.starts_with("outOfScope") {
                    V3Error::new("outOfScope", None, msg, StatusCode::FORBIDDEN)
                } else {
                    V3Error::new("serverError", None, msg, StatusCode::INTERNAL_SERVER_ERROR)
                });
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
                .bind(issued.quote.publication_id)
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

        let reserved_lines = issued
            .lines
            .into_iter()
            .map(|l| ReservedLineDto {
                quote_line_id: l.quote_line_id.to_string(),
                reserved_quantity: l.reserved_quantity.to_string(),
            })
            .collect();

        Ok(BindingQuoteIssueOutput {
            quote: QuoteDto::from_domain(issued.quote, pub_version),
            reserved_lines,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let service = Arc::new(IssueBindingQuote::new(
        state.db.clone(),
        make_approval_service(state.db.clone()),
    ));
    CommandEndpoint::register(cfg, Arc::new(BindingQuoteIssueCommand::new(service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BindingQuoteIssueInput::export_all(&cfg)?;
    ReservedLineDto::export_all(&cfg)?;
    BindingQuoteIssueOutput::export_all(&cfg)?;
    Ok(())
}
