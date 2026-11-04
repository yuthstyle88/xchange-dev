//! Backdated keying (`docs/design/backdated-keying.md` §4; the owner's rules R1–R4 and guard G of 2026-11-04):
//! `POST /api/transactions/backdated` keys one exchange at its real past time, with no shift, at the typed rate,
//! its cash in the branch SAFE. Its own policy row (`areas/backdated_keying.rs`, `ExchangeBackdate`) keeps tellers
//! out at the seam.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::repository::TransactionStore;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Bound, CurrencyCode, ForeignAmount, Money, Rate, Side, Sign};

use super::transactions::{TransactionPaymentInput, UploadRef};
use crate::application::workflows::exchange::backdate::{BackdateRequest, BackdateSale};
use crate::application::workflows::exchange::create::CreatePayment;
use crate::application::workflows::save_step::SaveActor;
use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::FileService;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::transaction::TransactionDto;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, role_not_allowed_here, server_error,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input::{self, TextBound};
use crate::presentation::http::v3::negative_stock::stock_refusal_answer;

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

fn invalid(field: &'static str, message: &str) -> V3Error {
    V3Error::new(
        "validationFailed",
        Some(field),
        message.to_owned(),
        StatusCode::BAD_REQUEST,
    )
}

fn refusal(e: AppError) -> V3Error {
    match e {
        AppError::Domain(ref m) if m.starts_with("dayClosed") => {
            V3Error::new("dayClosed", None, m.clone(), StatusCode::CONFLICT)
        }
        // The SAFE posting's raw marker becomes the shared 409: the Thai message plus the shortfall details
        // (`ledger-inventory.md` §5.1.2), as `complete.rs` and `reversal_post.rs` answer it.
        AppError::Domain(ref m) if m.starts_with("stockInsufficient") => stock_refusal_answer(m),
        other => super::transactions::refusal(other),
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
        let side = match input.side.as_str() {
            "BUY" => Side::Buy,
            "SELL" => Side::Sell,
            _ => return Err(invalid("side", "invalid side")),
        };
        let currency = CurrencyCode::parse(&input.currency)
            .map_err(|_| invalid("currency", "invalid currency"))?;
        let quantity = ForeignAmount::checked_from_decimal(input::decimal(
            "quantity",
            &input.quantity,
            Bound::numeric(20, 4, Sign::Positive).at_most(Money::MAX_INPUT_UNITS),
        )?)
        .map_err(|_| invalid("quantity", "invalid quantity"))?;
        let rate = Rate::parse(&input.rate).map_err(|_| invalid("rate", "invalid rate"))?;
        let fee_thb = match input.fee_thb.as_deref() {
            Some(s) => Money::parse(s).map_err(|_| invalid("feeThb", "invalid fee"))?,
            None => Money::ZERO,
        };
        let expected_settle_thb = match input.expected_settle_thb.as_deref() {
            Some(s) => Some(
                Money::parse(s)
                    .map_err(|_| invalid("expectedSettleThb", "invalid expectedSettleThb"))?,
            ),
            None => None,
        };
        let method = PaymentMethod::parse(&input.payment.method)
            .ok_or_else(|| invalid("payment.method", "invalid payment method"))?;
        let bank_fee_thb = Money::parse(&input.payment.bank_fee_thb)
            .map_err(|_| invalid("payment.bankFeeThb", "invalid bank fee"))?;
        let paid_at = DateTime::parse_from_rfc3339(&input.payment.paid_at)
            .map(|at| at.with_timezone(&Utc))
            .map_err(|_| invalid("payment.paidAt", "invalid paidAt"))?;
        let value_date = NaiveDate::parse_from_str(&input.value_date, "%Y-%m-%d")
            .map_err(|_| invalid("valueDate", "invalid valueDate"))?;
        input::text(
            "payment.reference",
            &input.payment.reference,
            TextBound { min: 0, max: 100 },
        )?;
        input::text(
            "payment.account",
            &input.payment.account,
            TextBound { min: 0, max: 100 },
        )?;
        input::text("notes", &input.notes, TextBound { min: 0, max: 500 })?;
        let req = BackdateRequest {
            branch_id: branch.clone(),
            customer_id: input.customer_id,
            side,
            currency,
            quantity,
            rate,
            fee_thb,
            expected_settle_thb,
            payment: CreatePayment {
                method,
                reference: input.payment.reference,
                account: input.payment.account,
                slip_key: input.payment.slip.map(|s| s.upload_key),
                bank_fee_thb,
                paid_at,
            },
            value_date,
            passport_key: input.passport.map(|p| p.upload_key),
            notes: input.notes,
        };
        let save_actor = SaveActor::from_actor(who).ok_or_else(role_not_allowed_here)?;
        let t = self
            .service
            .run_in(tx, req, &save_actor, Utc::now())
            .await
            .map_err(refusal)?;
        let mark = PgTransactionStore::new(self.pool.clone())
            .backdate_mark_in(tx, t.id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "transaction backdate mark read failed");
                server_error()
            })?;
        Ok(TransactionDto::from_domain(&t).with_backdate(mark))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The SAFE posting's raw `stockInsufficient` marker is answered as the shared 409 (`negative_stock.rs`): the
    /// Thai message and the shortfall details, never the internal string.
    #[test]
    fn a_short_safe_answers_the_shared_stock_refusal() {
        let raw = "stockInsufficient: the posting takes SAFE below zero; \
                   shortfall=[{\"currency\":\"THB\",\"place\":\"SAFE\",\"shortBy\":\"15975\"}]";
        let err = refusal(AppError::Domain(raw.to_owned()));
        assert_eq!(err.code, "stockInsufficient");
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert!(
            err.message.starts_with("ยอดเงินสดไม่พอ"),
            "the Thai shortfall message, not the marker: {}",
            err.message
        );
        assert!(!err.message.contains("shortfall=["), "{}", err.message);
        assert!(err.details.is_some(), "the shortfall details are attached");
    }

    #[test]
    fn a_closed_day_stays_409_day_closed() {
        let err = refusal(AppError::Domain(
            "dayClosed: business date 2026-11-01 is closed".to_owned(),
        ));
        assert_eq!(err.code, "dayClosed");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }
}
