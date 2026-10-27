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

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::application::workflows::exchange::binding_issue::IssueBindingQuote;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::quote::QuoteDto;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, Locks};
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
        let _ = (&self.service, tx, branch, who, input);
        todo!("BQ-5")
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
