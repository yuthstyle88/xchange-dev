//! Kiosk repository postgres implementation.

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side};

use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::repositories::kiosk_repository::{KioskRepository, StoredReceipt};
use crate::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use crate::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::domain::kiosk::value_objects::payment_method::PaymentMethod;
use crate::domain::kiosk::value_objects::receipt_image::ReceiptImage;
use crate::domain::kiosk::value_objects::transaction_link::TransactionLink;

type Tx<'a, 'c> = &'a mut sqlx::Transaction<'c, sqlx::Postgres>;

macro_rules! select_requests {
    ($tail:literal) => {
        concat!(
            "SELECT id, status, created_at, staff, side, currency::text AS currency, rate, \
             foreign_amount, thb, input_amount, payment_method, submission_id, document_ref \
             FROM kiosk_requests WHERE branch_id = $1 AND ",
            $tail
        )
    };
}

const SELECT_ONE: &str = select_requests!("id = $2");
const SELECT_QUOTED: &str = select_requests!("status = $2");
const SELECT_QUEUE: &str =
    select_requests!("status <> $2 ORDER BY created_at DESC, id DESC LIMIT $3");

#[derive(sqlx::FromRow)]
struct RequestRow {
    id: String,
    status: String,
    created_at: DateTime<Utc>,
    staff: Option<String>,
    side: String,
    currency: String,
    rate: Decimal,
    foreign_amount: Decimal,
    thb: Decimal,
    input_amount: Decimal,
    payment_method: String,
    submission_id: Option<String>,
    document_ref: Option<String>,
}

fn unreadable(what: &str, value: &dyn std::fmt::Display) -> AppError {
    AppError::Internal(format!(
        "kiosk_requests holds an unreadable {what}: {value}"
    ))
}

fn status_from(text: &str) -> AppResult<KioskStatus> {
    [
        KioskStatus::Quoted,
        KioskStatus::Waiting,
        KioskStatus::Accepted,
        KioskStatus::IdChecked,
        KioskStatus::Completed,
    ]
    .into_iter()
    .find(|status| status.as_str() == text)
    .ok_or_else(|| unreadable("status", &text))
}

fn side_text(side: Side) -> &'static str {
    match side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    }
}

fn side_from(text: &str) -> AppResult<Side> {
    match text {
        "BUY" => Ok(Side::Buy),
        "SELL" => Ok(Side::Sell),
        other => Err(unreadable("side", &other)),
    }
}

fn money_from(value: Decimal) -> AppResult<Money> {
    let mut hundredths = value;
    hundredths.rescale(2);
    i64::try_from(hundredths.mantissa())
        .ok()
        .and_then(|minor| Money::from_minor(minor).ok())
        .ok_or_else(|| unreadable("amount", &value))
}

fn money_to(money: Money) -> Decimal {
    Decimal::new(money.minor(), 2)
}

fn rate_from(value: Decimal) -> AppResult<Rate> {
    Rate::parse(&value.normalize().to_string()).map_err(|_| unreadable("rate", &value))
}

fn time_to(time: KioskTime) -> AppResult<DateTime<Utc>> {
    Utc.timestamp_millis_opt(time.epoch_millis())
        .single()
        .ok_or_else(|| AppError::Internal(format!("not a time: {} ms", time.epoch_millis())))
}

fn time_from(time: DateTime<Utc>) -> KioskTime {
    KioskTime::from_epoch_millis(time.timestamp_millis())
}

impl RequestRow {
    fn into_request(self) -> AppResult<KioskRequest> {
        let transaction = match (self.submission_id, self.document_ref) {
            (Some(submission_id), Some(document_ref)) => Some(TransactionLink {
                submission_id,
                document_ref,
            }),
            _ => None,
        };
        Ok(KioskRequest {
            status: status_from(&self.status)?,
            created: time_from(self.created_at),
            staff: self.staff,
            quote: KioskQuote {
                side: side_from(&self.side)?,
                currency: CurrencyCode::parse(self.currency.trim())
                    .map_err(|_| unreadable("currency", &self.currency))?,
                rate: rate_from(self.rate)?,
                foreign: money_from(self.foreign_amount)?,
                thb: money_from(self.thb)?,
                input_amount: money_from(self.input_amount)?,
                payment_method: PaymentMethod::parse(&self.payment_method)
                    .ok_or_else(|| unreadable("payment method", &self.payment_method))?,
            },
            transaction,
            id: self.id,
        })
    }
}

fn into_requests(rows: Vec<RequestRow>) -> AppResult<Vec<KioskRequest>> {
    rows.into_iter().map(RequestRow::into_request).collect()
}

fn into_switches(rows: Vec<(String, bool)>) -> AppResult<Vec<CurrencySwitch>> {
    rows.into_iter()
        .map(|(code, enabled)| {
            CurrencyCode::parse(code.trim())
                .map(|code| CurrencySwitch { code, enabled })
                .map_err(|_| {
                    AppError::Internal(format!("kiosk_currencies holds an unknown code: {code}"))
                })
        })
        .collect()
}

#[derive(Clone)]
pub struct PgKioskRepository {
    pool: PgPool,
}

impl PgKioskRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const SELECT_SWITCHES: &str =
    "SELECT code::text, enabled FROM kiosk_currencies WHERE branch_id = $1 ORDER BY code";

impl KioskRepository for PgKioskRepository {
    async fn find(&self, branch: &BranchId, id: &str) -> AppResult<Option<KioskRequest>> {
        sqlx::query_as::<_, RequestRow>(SELECT_ONE)
            .bind(branch.as_str())
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .map(RequestRow::into_request)
            .transpose()
    }

    async fn find_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
    ) -> AppResult<Option<KioskRequest>> {
        sqlx::query_as::<_, RequestRow>(SELECT_ONE)
            .bind(branch.as_str())
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?
            .map(RequestRow::into_request)
            .transpose()
    }

    async fn insert_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        request: &KioskRequest,
        client_ip: &str,
    ) -> AppResult<()> {
        let link = request.transaction.as_ref();
        sqlx::query(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, \
             rate, foreign_amount, thb, input_amount, payment_method, client_ip, submission_id, \
             document_ref) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(branch.as_str())
        .bind(&request.id)
        .bind(request.status.as_str())
        .bind(time_to(request.created)?)
        .bind(request.staff.as_deref())
        .bind(side_text(request.quote.side))
        .bind(request.quote.currency.as_str())
        .bind(request.quote.rate.value())
        .bind(money_to(request.quote.foreign))
        .bind(money_to(request.quote.thb))
        .bind(money_to(request.quote.input_amount))
        .bind(request.quote.payment_method.as_str())
        .bind(client_ip)
        .bind(link.map(|l| l.submission_id.as_str()))
        .bind(link.map(|l| l.document_ref.as_str()))
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn update_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        request: &KioskRequest,
    ) -> AppResult<()> {
        let link = request.transaction.as_ref();
        let done = sqlx::query(
            "UPDATE kiosk_requests SET status = $3, staff = $4, submission_id = $5, document_ref = $6 \
             WHERE branch_id = $1 AND id = $2",
        )
        .bind(branch.as_str())
        .bind(&request.id)
        .bind(request.status.as_str())
        .bind(request.staff.as_deref())
        .bind(link.map(|l| l.submission_id.as_str()))
        .bind(link.map(|l| l.document_ref.as_str()))
        .execute(&mut **tx)
        .await?;
        if done.rows_affected() != 1 {
            return Err(AppError::Internal(format!(
                "kiosk request {} of branch {branch} is gone",
                request.id
            )));
        }
        Ok(())
    }

    async fn quoted_in(&self, tx: Tx<'_, '_>, branch: &BranchId) -> AppResult<Vec<KioskRequest>> {
        let rows = sqlx::query_as::<_, RequestRow>(SELECT_QUOTED)
            .bind(branch.as_str())
            .bind(KioskStatus::Quoted.as_str())
            .fetch_all(&mut **tx)
            .await?;
        into_requests(rows)
    }

    async fn delete_in(&self, tx: Tx<'_, '_>, branch: &BranchId, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM kiosk_requests WHERE branch_id = $1 AND id = $2")
            .bind(branch.as_str())
            .bind(id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn quotes_from_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        client_ip: &str,
        since: KioskTime,
    ) -> AppResult<i64> {
        let count = sqlx::query_scalar(
            "SELECT count(*) FROM kiosk_requests \
             WHERE branch_id = $1 AND client_ip = $2 AND created_at > $3",
        )
        .bind(branch.as_str())
        .bind(client_ip)
        .bind(time_to(since)?)
        .fetch_one(&mut **tx)
        .await?;
        Ok(count)
    }

    async fn waiting(&self, branch: &BranchId) -> AppResult<(i64, Option<String>)> {
        let row: (i64, Option<String>) = sqlx::query_as(
            "SELECT count(*), (array_agg(id ORDER BY created_at DESC, id DESC))[1] \
             FROM kiosk_requests WHERE branch_id = $1 AND status = $2",
        )
        .bind(branch.as_str())
        .bind(KioskStatus::Waiting.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    async fn queue(&self, branch: &BranchId, limit: i64) -> AppResult<Vec<KioskRequest>> {
        let rows = sqlx::query_as::<_, RequestRow>(SELECT_QUEUE)
            .bind(branch.as_str())
            .bind(KioskStatus::Quoted.as_str())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        into_requests(rows)
    }

    async fn switches(&self, branch: &BranchId) -> AppResult<Vec<CurrencySwitch>> {
        let rows = sqlx::query_as(SELECT_SWITCHES)
            .bind(branch.as_str())
            .fetch_all(&self.pool)
            .await?;
        into_switches(rows)
    }

    async fn switches_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
    ) -> AppResult<Vec<CurrencySwitch>> {
        let rows = sqlx::query_as(SELECT_SWITCHES)
            .bind(branch.as_str())
            .fetch_all(&mut **tx)
            .await?;
        into_switches(rows)
    }

    async fn set_switch_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        code: CurrencyCode,
        enabled: bool,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO kiosk_currencies (branch_id, code, enabled) VALUES ($1, $2, $3) \
             ON CONFLICT (branch_id, code) DO UPDATE SET enabled = excluded.enabled",
        )
        .bind(branch.as_str())
        .bind(code.as_str())
        .bind(enabled)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn receipt(&self, branch: &BranchId, id: &str) -> AppResult<Option<StoredReceipt>> {
        let row: Option<(String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT image, updated_at FROM kiosk_receipts WHERE branch_id = $1 AND request_id = $2",
        )
        .bind(branch.as_str())
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(image, updated_at)| StoredReceipt {
            image,
            published: time_from(updated_at),
        }))
    }

    async fn put_receipt_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        image: &ReceiptImage,
        at: KioskTime,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO kiosk_receipts (branch_id, request_id, image, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (branch_id, request_id) \
             DO UPDATE SET image = excluded.image, updated_at = excluded.updated_at",
        )
        .bind(branch.as_str())
        .bind(id)
        .bind(image.as_str())
        .bind(time_to(at)?)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}
