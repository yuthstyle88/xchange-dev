//! Transaction repository postgres implementation.

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Money, Rate, Side, plain_decimal};

use crate::domain::customer::value_objects::mode::Mode;
use crate::domain::exchange::entities::transaction::Transaction;
use crate::domain::exchange::repositories::transaction_repository::TransactionRepository;
use crate::domain::exchange::value_objects::payment_method::PaymentMethod;

#[derive(Clone)]
pub struct PgTransactionRepository {
    pool: PgPool,
}

const SELECT: &str = "SELECT COALESCE(t.document_ref, '') AS document_ref, t.branch_id, t.mode, t.shift_id, t.submission_id, t.customer_id, \
    t.customer_name, t.nationality, l.shop_side AS type, TRIM(l.currency)::text AS code, l.quantity::numeric(20,2)::text AS amount, l.rate_snapshot::text AS rate, \
    l.reference_rate::text AS reference_rate, l.thb_amount::text AS principal, l.fee_thb::text AS fee, \
    p.bank_fee_thb::text AS bank_fee, \
    CASE WHEN l.shop_side = 'SELL' THEN l.settle_thb::text ELSE '0.00' END AS received, \
    CASE WHEN l.shop_side = 'BUY' THEN l.settle_thb::text ELSE '0.00' END AS paid, \
    p.method, p.reference AS payment_ref, \
    p.account, t.staff_id, t.booking_no, t.notes, t.value_date, t.paid_at, t.recorded_at, t.recorded_by, t.payload_hash, \
    COALESCE((SELECT file_key FROM transaction_files f WHERE f.transaction_id = t.id AND f.kind = 'PASSPORT'), '') AS passport_key, \
    p.slip_file_key AS slip_key, \
    COALESCE((SELECT file_key FROM transaction_files f WHERE f.transaction_id = t.id AND f.kind = 'RECEIPT'), '') AS receipt_key \
    FROM transactions t \
    JOIN transaction_lines l ON l.transaction_id = t.id \
    JOIN transaction_payments p ON p.transaction_id = t.id";

fn bad(what: &str) -> AppError {
    AppError::Internal(format!("stored transaction: {what}"))
}

fn money(row: &sqlx::postgres::PgRow, col: &str) -> AppResult<Money> {
    let text: String = row.try_get(col).map_err(AppError::Database)?;
    let mut d = plain_decimal(&text).ok_or_else(|| bad(col))?;
    if d.scale() > 2 {
        return Err(bad(col));
    }
    d.rescale(2);
    Money::try_from_minor_i128(d.mantissa()).map_err(|_| bad(col))
}

fn map(row: &sqlx::postgres::PgRow) -> AppResult<(Transaction, String)> {
    let get = |c: &str| -> AppResult<String> { row.try_get(c).map_err(AppError::Database) };
    let mode = if get("mode")? == "DEMO" {
        Mode::Demo
    } else {
        Mode::Live
    };
    let side = if get("type")? == "SELL" {
        Side::Sell
    } else {
        Side::Buy
    };
    let method_text = get("method")?;
    let method = [
        PaymentMethod::Cash,
        PaymentMethod::ScbTransfer,
        PaymentMethod::BankTransfer,
        PaymentMethod::CreditCard,
        PaymentMethod::Alipay,
        PaymentMethod::PromptPay,
    ]
    .into_iter()
    .find(|m| m.as_str() == method_text)
    .ok_or_else(|| bad("method"))?;
    let reference_rate: Option<String> =
        row.try_get("reference_rate").map_err(AppError::Database)?;
    let value_date: NaiveDate = row.try_get("value_date").map_err(AppError::Database)?;
    let paid_at: DateTime<Utc> = row.try_get("paid_at").map_err(AppError::Database)?;
    let recorded_at: DateTime<Utc> = row.try_get("recorded_at").map_err(AppError::Database)?;
    Ok((
        Transaction {
            document_ref: get("document_ref")?,
            branch_id: BranchId::new(get("branch_id")?),
            mode,
            shift_id: row.try_get("shift_id").map_err(AppError::Database)?,
            submission_id: get("submission_id")?,
            customer_id: get("customer_id")?,
            customer_name: get("customer_name")?,
            nationality: get("nationality")?,
            side,
            currency: get("code")?,
            amount: money(row, "amount")?,
            rate: Rate::parse(&get("rate")?).map_err(|_| bad("rate"))?,
            reference_rate: match reference_rate {
                Some(t) => Some(Rate::parse(&t).map_err(|_| bad("reference_rate"))?),
                None => None,
            },
            principal: money(row, "principal")?,
            fee: money(row, "fee")?,
            bank_fee: money(row, "bank_fee")?,
            received: money(row, "received")?,
            paid: money(row, "paid")?,
            method,
            payment_ref: get("payment_ref")?,
            account: get("account")?,
            staff: get("staff_id")?,
            booking: get("booking_no")?,
            notes: get("notes")?,
            value_date,
            paid_at,
            passport_key: get("passport_key")?,
            slip_key: row.try_get("slip_key").map_err(AppError::Database)?,
            receipt_key: get("receipt_key")?,
            recorded_at,
            recorded_by: get("recorded_by")?,
        },
        get("payload_hash")?,
    ))
}

impl PgTransactionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn organization_of_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
    ) -> AppResult<uuid::Uuid> {
        sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)
    }
}

impl TransactionRepository for PgTransactionRepository {
    async fn find(&self, branch: &BranchId, document_ref: &str) -> AppResult<Option<Transaction>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} WHERE t.branch_id = $1 AND t.document_ref = $2 AND t.status = 'COMPLETED'"
        )))
        .bind(branch.as_str())
        .bind(document_ref)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;
        row.as_ref().map(|r| map(r).map(|x| x.0)).transpose()
    }

    async fn list(&self, branch: &BranchId, mode: Mode) -> AppResult<Vec<Transaction>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} WHERE t.branch_id = $1 AND t.mode = $2 AND t.status = 'COMPLETED' ORDER BY t.seq, l.line_no"
        )))
        .bind(branch.as_str())
        .bind(mode.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        rows.iter().map(|r| map(r).map(|x| x.0)).collect()
    }

    async fn find_submission_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        mode: Mode,
        submission_id: &'a str,
    ) -> AppResult<Option<(Transaction, String)>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} WHERE t.branch_id = $1 AND t.mode = $2 AND t.submission_id = $3 AND t.status = 'COMPLETED'"
        )))
        .bind(branch.as_str())
        .bind(mode.as_str())
        .bind(submission_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        row.as_ref().map(map).transpose()
    }

    async fn next_number_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        counter_key: &'a str,
    ) -> AppResult<i64> {
        sqlx::query_scalar(
            "INSERT INTO counters (key, value) VALUES ($1, 1) \
             ON CONFLICT (key) DO UPDATE SET value = counters.value + 1 RETURNING value",
        )
        .bind(counter_key)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)
    }

    async fn insert_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        t: &'a Transaction,
        payload_hash: &'a str,
    ) -> AppResult<()> {
        let org = Self::organization_of_in(tx, &t.branch_id).await?;

        let unit_size = rust_decimal::Decimal::ONE;

        let tx_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
             customer_id, customer_name, nationality, type, code, amount, rate, reference_rate, principal, fee, \
             bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, value_date, \
             paid_at, recorded_at, recorded_by, organization_id, status, completed_at, unit_size) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, \
             $12::numeric, $13::numeric, $14::numeric, $15::numeric, $16::numeric, $17::numeric, $18::numeric, \
             $19::numeric, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, 'COMPLETED', $28, $31) RETURNING id",
        )
        .bind(&t.document_ref)
        .bind(t.branch_id.as_str())
        .bind(t.mode.as_str())
        .bind(&t.shift_id)
        .bind(&t.submission_id)
        .bind(payload_hash)
        .bind(&t.customer_id)
        .bind(&t.customer_name)
        .bind(&t.nationality)
        .bind(match t.side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        })
        .bind(&t.currency)
        .bind(t.amount.to_string())
        .bind(t.rate.to_string())
        .bind(t.reference_rate.map(|r| r.to_string()))
        .bind(t.principal.to_string())
        .bind(t.fee.to_string())
        .bind(t.bank_fee.to_string())
        .bind(t.received.to_string())
        .bind(t.paid.to_string())
        .bind(t.method.as_str())
        .bind(&t.payment_ref)
        .bind(&t.account)
        .bind(&t.staff)
        .bind(&t.booking)
        .bind(&t.notes)
        .bind(t.value_date)
        .bind(t.paid_at)
        .bind(t.recorded_at)
        .bind(&t.recorded_by)
        .bind(org)
        .bind(unit_size)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let side_str = match t.side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        };
        let settle_thb = match t.side {
            Side::Buy => t.paid.to_string(),
            Side::Sell => t.received.to_string(),
        };

        let line_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transaction_lines ( \
                id, organization_id, transaction_id, quote_line_id, shop_side, currency, \
                denomination_id, quantity, unit_size, rate_snapshot, reference_rate, \
                thb_amount, fee_thb, settle_thb, cost_basis_thb, realized_margin_thb, \
                line_no, created_at, updated_at \
            ) VALUES ( \
                $1, $2, $3, NULL, $4, $5, \
                NULL, $6::numeric, $7::numeric, $8::numeric, $9::numeric, \
                $10::numeric, $11::numeric, $12::numeric, NULL, NULL, \
                1, $13, $13 \
            )",
        )
        .bind(line_id)
        .bind(org)
        .bind(tx_id)
        .bind(side_str)
        .bind(&t.currency)
        .bind(t.amount.to_string())
        .bind(unit_size)
        .bind(t.rate.to_string())
        .bind(t.reference_rate.map(|r| r.to_string()))
        .bind(t.principal.to_string())
        .bind(t.fee.to_string())
        .bind(&settle_thb)
        .bind(t.recorded_at)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let payment_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transaction_payments ( \
                id, organization_id, transaction_id, payment_no, method, reference, \
                account, slip_file_key, amount_thb, bank_fee_thb, paid_at, \
                created_at, updated_at \
            ) VALUES ( \
                $1, $2, $3, 1, $4, $5, \
                $6, $7, $8::numeric, $9::numeric, $10, \
                $11, $11 \
            )",
        )
        .bind(payment_id)
        .bind(org)
        .bind(tx_id)
        .bind(t.method.as_str())
        .bind(&t.payment_ref)
        .bind(&t.account)
        .bind(t.slip_key.as_deref())
        .bind(&settle_thb)
        .bind(t.bank_fee.to_string())
        .bind(t.paid_at)
        .bind(t.recorded_at)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let receipt_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO receipts ( \
                id, organization_id, transaction_id, number, revision, issued_at, document_ref, \
                created_at, updated_at \
            ) VALUES ( \
                $1, $2, $3, $4, 1, $5, $6, \
                $5, $5 \
            )",
        )
        .bind(receipt_id)
        .bind(org)
        .bind(tx_id)
        .bind(&t.document_ref)
        .bind(t.recorded_at)
        .bind(&t.receipt_key)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let mut files = vec![
            ("PASSPORT", t.passport_key.as_str()),
            ("RECEIPT", t.receipt_key.as_str()),
        ];
        if let Some(slip) = &t.slip_key {
            files.push(("SLIP", slip.as_str()));
        }
        for (kind, key) in files {
            sqlx::query(
                "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, $2, $3)",
            )
            .bind(tx_id)
            .bind(kind)
            .bind(key)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;
        }
        Ok(())
    }
}
