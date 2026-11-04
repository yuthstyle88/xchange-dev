//! Backdated keying (`docs/design/backdated-keying.md` §4; the owner's rules R1–R4 and guard G of 2026-11-04):
//! `POST /api/transactions/backdated` keys one exchange at its real past time, with no shift, at the typed rate,
//! its cash in the branch SAFE. Its own policy row (`areas/backdated_keying.rs`, `ExchangeBackdate`) keeps tellers
//! out at the seam.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use super::transactions::{TransactionPaymentInput, UploadRef};
use crate::application::workflows::exchange::backdate::BackdateSale;
use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::FileService;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::transaction::TransactionDto;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

/// `[required]` §4's body. Every figure is the caller's.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction_backdate.ts")]
pub struct TransactionBackdateInput {
    /// An existing LIVE customer of this branch.
    pub customer_id: String,
    /// The shop's side: `"BUY"` | `"SELL"`.
    pub side: String,
    /// A currency code other than THB.
    pub currency: String,
    /// The foreign amount, > 0, at the currency's minor unit.
    pub quantity: String,
    /// THB per the branch's `unit_size` for `currency`, as the shop used it; > 0, at most 8 decimal places.
    pub rate: String,
    /// Default `"0"`.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub fee_thb: Option<String>,
    /// When sent, must equal the computed settle figure (§6).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub expected_settle_thb: Option<String>,
    /// `paidAt` is the deal's real time (RFC 3339).
    pub payment: TransactionPaymentInput,
    /// The business date, `YYYY-MM-DD`: `paidAt`'s date in the branch's timezone.
    pub value_date: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub passport: Option<UploadRef>,
    #[serde(default)]
    pub notes: String,
}

pub struct TransactionBackdateCommand {
    service: BackdateSale,
    pool: PgPool,
}

impl TransactionBackdateCommand {
    pub fn new(pool: PgPool, files: FileService) -> Self {
        Self {
            service: BackdateSale::new(pool.clone(), files),
            pool,
        }
    }
}

impl CommandAction for TransactionBackdateCommand {
    const NAME: &'static str = "transactionBackdate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transactionBackdate");
    type Input = TransactionBackdateInput;
    type Output = TransactionDto;
    const ROUTE: &'static str = "/api/transactions/backdated";
    const METHOD: Method = Method::POST;

    /// `[required]` §4: parse the body (each bad field `400 validationFailed` naming it: `side`, `currency`,
    /// `quantity`, `rate`, `feeThb`, `expectedSettleThb`, `payment.method`, `payment.bankFeeThb`, `payment.paidAt`,
    /// `valueDate`, and the text bounds of `transactions.rs`), build a `BackdateRequest`, run
    /// `BackdateSale::run_in(tx, req, &SaveActor::from_actor(who), Utc::now())`, map its refusals with
    /// `transactions::refusal` (`dayClosed` → 409), and answer `TransactionDto::from_domain(&t)
    /// .with_backdate(store.backdate_mark_in(..))`.
    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (&self.service, &self.pool, tx, branch, who, input);
        todo!("BK-9: backdated-keying.md §4")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    let files = FileService::new(pool.clone(), state.settings.files.dir.clone());
    CommandEndpoint::register(cfg, Arc::new(TransactionBackdateCommand::new(pool, files)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    TransactionBackdateInput::export_all(&cfg)?;
    Ok(())
}
