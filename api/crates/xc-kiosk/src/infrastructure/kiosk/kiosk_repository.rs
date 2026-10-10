//! Kiosk repository postgres implementation.

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};

use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::repositories::kiosk_repository::{KioskRepository, StoredReceipt};
use crate::domain::kiosk::value_objects::counter_facts::LineId;
use crate::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use crate::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::domain::kiosk::value_objects::notifications::WorkingRequest;
use crate::domain::kiosk::value_objects::payment_method::PaymentMethod;
use crate::domain::kiosk::value_objects::queue_filter::QueueFilter;
use crate::domain::kiosk::value_objects::receipt_image::ReceiptImage;
use crate::domain::kiosk::value_objects::transaction_link::TransactionLink;
use crate::domain::kiosk::value_objects::work_times::WorkTimes;

type Tx<'a, 'c> = &'a mut sqlx::Transaction<'c, sqlx::Postgres>;

macro_rules! select_requests {
    ($tail:literal) => {
        concat!(
            "SELECT id, status, created_at, staff, side, currency::text AS currency, rate, \
             unit_size, published_rate_line_id::text AS published_rate_line_id, \
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

macro_rules! queue_queries {
    (@sql $pred:literal, $keyset:literal, $limit:literal) => {
        concat!(
            "SELECT id, status, created_at, staff, side, currency::text AS currency, rate, \
             unit_size, published_rate_line_id::text AS published_rate_line_id, \
             foreign_amount, thb, input_amount, payment_method, submission_id, document_ref, \
             started_at, completed_at \
             FROM kiosk_requests WHERE branch_id = $1 AND ",
            $pred,
            $keyset,
            " ORDER BY created_at DESC, id DESC LIMIT ",
            $limit
        )
    };
    ($(($page:ident, $first:ident, $before:ident, $pred:literal)),* $(,)?) => {
        $(
            const $page: &str = queue_queries!(@sql $pred, "", "51 OFFSET $2");
            const $first: &str = queue_queries!(@sql $pred, "", "$2");
            const $before: &str = queue_queries!(
                @sql $pred,
                " AND (created_at, id) < (SELECT created_at, id FROM kiosk_requests WHERE branch_id = $1 AND id = $2)",
                "$3"
            );
        )*
    };
}

queue_queries!(
    (
        SELECT_QUEUE_PAGE_ALL,
        SELECT_QUEUE_FIRST_ALL,
        SELECT_QUEUE_BEFORE_ALL,
        "status <> 'QUOTED'"
    ),
    (
        SELECT_QUEUE_PAGE_WAITING,
        SELECT_QUEUE_FIRST_WAITING,
        SELECT_QUEUE_BEFORE_WAITING,
        "status = 'WAITING'"
    ),
    (
        SELECT_QUEUE_PAGE_WORKING,
        SELECT_QUEUE_FIRST_WORKING,
        SELECT_QUEUE_BEFORE_WORKING,
        "status IN ('ACCEPTED', 'ID_CHECKED')"
    ),
    (
        SELECT_QUEUE_PAGE_COMPLETED,
        SELECT_QUEUE_FIRST_COMPLETED,
        SELECT_QUEUE_BEFORE_COMPLETED,
        "status = 'COMPLETED'"
    ),
);

/// The statuses each queue filter reads. `All` = every status but QUOTED (the CHECK of
/// 20261007170000_kiosk.sql allows exactly these five).
pub fn queue_statuses(filter: QueueFilter) -> &'static [&'static str] {
    match filter {
        QueueFilter::All => &["WAITING", "ACCEPTED", "ID_CHECKED", "COMPLETED"],
        QueueFilter::Waiting => &["WAITING"],
        QueueFilter::Working => &["ACCEPTED", "ID_CHECKED"],
        QueueFilter::Completed => &["COMPLETED"],
    }
}

/// The literal SQL status predicate for each queue filter.
pub fn queue_predicate(filter: QueueFilter) -> &'static str {
    match filter {
        QueueFilter::All => "status <> 'QUOTED'",
        QueueFilter::Waiting => "status = 'WAITING'",
        QueueFilter::Working => "status IN ('ACCEPTED', 'ID_CHECKED')",
        QueueFilter::Completed => "status = 'COMPLETED'",
    }
}

#[derive(sqlx::FromRow)]
struct RequestRow {
    id: String,
    status: String,
    created_at: DateTime<Utc>,
    staff: Option<String>,
    side: String,
    currency: String,
    rate: Decimal,
    unit_size: Decimal,
    published_rate_line_id: Option<String>,
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
                unit_size: UnitSize::from_decimal(self.unit_size)
                    .map_err(|_| unreadable("unit size", &self.unit_size))?,
                line_id: self
                    .published_rate_line_id
                    .as_deref()
                    .and_then(LineId::parse_uuid),
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

#[derive(sqlx::FromRow)]
struct QueueRow {
    id: String,
    status: String,
    created_at: DateTime<Utc>,
    staff: Option<String>,
    side: String,
    currency: String,
    rate: Decimal,
    unit_size: Decimal,
    published_rate_line_id: Option<String>,
    foreign_amount: Decimal,
    thb: Decimal,
    input_amount: Decimal,
    payment_method: String,
    submission_id: Option<String>,
    document_ref: Option<String>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
}

impl QueueRow {
    fn into_request_and_times(self) -> AppResult<(KioskRequest, WorkTimes)> {
        let times = WorkTimes {
            started_at: self.started_at.map(time_from),
            completed_at: self.completed_at.map(time_from),
        };
        let row = RequestRow {
            id: self.id,
            status: self.status,
            created_at: self.created_at,
            staff: self.staff,
            side: self.side,
            currency: self.currency,
            rate: self.rate,
            unit_size: self.unit_size,
            published_rate_line_id: self.published_rate_line_id,
            foreign_amount: self.foreign_amount,
            thb: self.thb,
            input_amount: self.input_amount,
            payment_method: self.payment_method,
            submission_id: self.submission_id,
            document_ref: self.document_ref,
        };
        Ok((row.into_request()?, times))
    }
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
        let line_uuid_str = request.quote.line_id.map(|l| l.to_string());
        sqlx::query(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, \
             rate, unit_size, published_rate_line_id, foreign_amount, thb, input_amount, payment_method, client_ip, submission_id, \
             document_ref) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::uuid, $11, $12, $13, $14, $15, $16, $17)",
        )
        .bind(branch.as_str())
        .bind(&request.id)
        .bind(request.status.as_str())
        .bind(time_to(request.created)?)
        .bind(request.staff.as_deref())
        .bind(side_text(request.quote.side))
        .bind(request.quote.currency.as_str())
        .bind(request.quote.rate.value())
        .bind(request.quote.unit_size.value())
        .bind(line_uuid_str)
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

    async fn queue_page(
        &self,
        branch: &BranchId,
        filter: QueueFilter,
        page: u32,
    ) -> AppResult<Vec<(KioskRequest, WorkTimes)>> {
        let offset = (page as i64) * 50;
        let sql = match filter {
            QueueFilter::All => SELECT_QUEUE_PAGE_ALL,
            QueueFilter::Waiting => SELECT_QUEUE_PAGE_WAITING,
            QueueFilter::Working => SELECT_QUEUE_PAGE_WORKING,
            QueueFilter::Completed => SELECT_QUEUE_PAGE_COMPLETED,
        };
        let rows = sqlx::query_as::<_, QueueRow>(sql)
            .bind(branch.as_str())
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(QueueRow::into_request_and_times)
            .collect()
    }

    async fn queue_before(
        &self,
        branch: &BranchId,
        filter: QueueFilter,
        before: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<(KioskRequest, WorkTimes)>> {
        let rows = if let Some(before_id) = before {
            let sql = match filter {
                QueueFilter::All => SELECT_QUEUE_BEFORE_ALL,
                QueueFilter::Waiting => SELECT_QUEUE_BEFORE_WAITING,
                QueueFilter::Working => SELECT_QUEUE_BEFORE_WORKING,
                QueueFilter::Completed => SELECT_QUEUE_BEFORE_COMPLETED,
            };
            sqlx::query_as::<_, QueueRow>(sql)
                .bind(branch.as_str())
                .bind(before_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        } else {
            let sql = match filter {
                QueueFilter::All => SELECT_QUEUE_FIRST_ALL,
                QueueFilter::Waiting => SELECT_QUEUE_FIRST_WAITING,
                QueueFilter::Working => SELECT_QUEUE_FIRST_WORKING,
                QueueFilter::Completed => SELECT_QUEUE_FIRST_COMPLETED,
            };
            sqlx::query_as::<_, QueueRow>(sql)
                .bind(branch.as_str())
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };

        rows.into_iter()
            .map(QueueRow::into_request_and_times)
            .collect()
    }

    async fn working(
        &self,
        branch: &BranchId,
        limit: i64,
    ) -> AppResult<(Vec<WorkingRequest>, i64)> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kiosk_requests \
             WHERE branch_id = $1 AND status IN ('ACCEPTED', 'ID_CHECKED')",
        )
        .bind(branch.as_str())
        .fetch_one(&self.pool)
        .await?;

        #[derive(sqlx::FromRow)]
        struct WorkingRow {
            id: String,
            staff: Option<String>,
            started_at: Option<DateTime<Utc>>,
            status: String,
        }

        let rows = sqlx::query_as::<_, WorkingRow>(
            "SELECT id, staff, started_at, status FROM kiosk_requests \
             WHERE branch_id = $1 AND status IN ('ACCEPTED', 'ID_CHECKED') \
             ORDER BY started_at ASC NULLS LAST, id ASC LIMIT $2",
        )
        .bind(branch.as_str())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut working = Vec::with_capacity(rows.len());
        for row in rows {
            working.push(WorkingRequest {
                id: row.id,
                staff: row.staff.unwrap_or_default(),
                started_at: row.started_at.map(time_from),
                status: status_from(&row.status)?,
            });
        }
        Ok((working, count))
    }

    async fn mark_started_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        at: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query("UPDATE kiosk_requests SET started_at = $3 WHERE branch_id = $1 AND id = $2")
            .bind(branch.as_str())
            .bind(id)
            .bind(at)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn mark_completed_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        at: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE kiosk_requests SET completed_at = COALESCE(completed_at, $3) WHERE branch_id = $1 AND id = $2",
        )
        .bind(branch.as_str())
        .bind(id)
        .bind(at)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn times_in(&self, tx: Tx<'_, '_>, branch: &BranchId, id: &str) -> AppResult<WorkTimes> {
        #[derive(sqlx::FromRow)]
        struct TimesRow {
            started_at: Option<DateTime<Utc>>,
            completed_at: Option<DateTime<Utc>>,
        }

        let row = sqlx::query_as::<_, TimesRow>(
            "SELECT started_at, completed_at FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
        )
        .bind(branch.as_str())
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
        match row {
            Some(r) => Ok(WorkTimes {
                started_at: r.started_at.map(time_from),
                completed_at: r.completed_at.map(time_from),
            }),
            None => Ok(WorkTimes::default()),
        }
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

    async fn published_both_sides_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        code: CurrencyCode,
    ) -> AppResult<bool> {
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS( \
                SELECT 1 FROM branches b \
                JOIN rate_publications rp ON rp.id = b.current_publication_id \
                JOIN published_rate_lines prl ON prl.publication_id = rp.id \
                WHERE b.id = $1 \
                  AND prl.currency_code = $2 \
                  AND prl.denomination_group = 'ALL' \
                  AND prl.buy_rate IS NOT NULL \
                  AND prl.sell_rate IS NOT NULL \
            )",
        )
        .bind(branch.as_str())
        .bind(code.as_str())
        .fetch_one(&mut **tx)
        .await?;
        Ok(exists)
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

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_DB_STATUSES: &[&str] = &["QUOTED", "WAITING", "ACCEPTED", "ID_CHECKED", "COMPLETED"];

    #[test]
    fn each_const_predicate_matches_queue_statuses() {
        let filters = [
            (
                QueueFilter::All,
                SELECT_QUEUE_PAGE_ALL,
                SELECT_QUEUE_FIRST_ALL,
                SELECT_QUEUE_BEFORE_ALL,
            ),
            (
                QueueFilter::Waiting,
                SELECT_QUEUE_PAGE_WAITING,
                SELECT_QUEUE_FIRST_WAITING,
                SELECT_QUEUE_BEFORE_WAITING,
            ),
            (
                QueueFilter::Working,
                SELECT_QUEUE_PAGE_WORKING,
                SELECT_QUEUE_FIRST_WORKING,
                SELECT_QUEUE_BEFORE_WORKING,
            ),
            (
                QueueFilter::Completed,
                SELECT_QUEUE_PAGE_COMPLETED,
                SELECT_QUEUE_FIRST_COMPLETED,
                SELECT_QUEUE_BEFORE_COMPLETED,
            ),
        ];

        for (filter, page_sql, first_sql, before_sql) in filters {
            let pred = queue_predicate(filter);
            let expected_statuses = queue_statuses(filter);

            // 1. Verify all 3 queries contain the exact predicate written beside queue_statuses
            for sql in [page_sql, first_sql, before_sql] {
                assert!(
                    sql.contains(pred),
                    "Query {sql} must contain predicate {pred}"
                );
            }

            // 2. Verify that evaluating the SQL predicate over all database statuses
            // matches the statuses defined in queue_statuses.
            let matched_statuses: Vec<&str> = ALL_DB_STATUSES
                .iter()
                .copied()
                .filter(|&status| {
                    if pred == "status <> 'QUOTED'" {
                        status != "QUOTED"
                    } else if let Some(exact) = pred
                        .strip_prefix("status = '")
                        .and_then(|s| s.strip_suffix('\''))
                    {
                        status == exact
                    } else if pred.starts_with("status IN (") {
                        pred.contains(&format!("'{status}'"))
                    } else {
                        panic!("unrecognized predicate format: {pred}");
                    }
                })
                .collect();

            assert_eq!(
                matched_statuses, expected_statuses,
                "Predicate '{pred}' must match queue_statuses for {filter:?}"
            );
        }
    }
}
