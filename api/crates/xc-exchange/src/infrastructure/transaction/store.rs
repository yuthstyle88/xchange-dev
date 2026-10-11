//! PgTransactionStore implementing TransactionStore.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize, plain_decimal};

use crate::domain::customer::value_objects::mode::Mode;
use crate::domain::exchange::value_objects::document_ref::DocumentRef;
use crate::domain::exchange::value_objects::payment_method::PaymentMethod;
use crate::domain::transaction::line::TransactionLine;
use crate::domain::transaction::payment::TransactionPayment;
use crate::domain::transaction::receipt_record::Receipt;
use crate::domain::transaction::repository::{StatusMove, TransactionStore};
use crate::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};

#[derive(Clone)]
pub struct PgTransactionStore {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgTransactionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn parse_payment_method(text: &str) -> AppResult<PaymentMethod> {
    [
        PaymentMethod::Cash,
        PaymentMethod::ScbTransfer,
        PaymentMethod::BankTransfer,
        PaymentMethod::CreditCard,
        PaymentMethod::Alipay,
        PaymentMethod::PromptPay,
    ]
    .into_iter()
    .find(|m| m.as_str() == text)
    .ok_or_else(|| AppError::Internal(format!("unknown payment method: {text}")))
}

impl TransactionStore for PgTransactionStore {
    async fn insert_draft_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        t: &'a ExchangeTransaction,
    ) -> AppResult<()> {
        if t.status != TransactionStatus::Draft {
            return Err(AppError::Domain(
                "validationFailed: status must be DRAFT".into(),
            ));
        }
        if t.document_ref.is_some() {
            return Err(AppError::Domain(
                "validationFailed: draft cannot have a document_ref".into(),
            ));
        }
        if !t.receipts.is_empty() {
            return Err(AppError::Domain(
                "validationFailed: draft cannot have receipts".into(),
            ));
        }
        if t.lines.len() != 1 {
            return Err(AppError::Domain(
                "validationFailed: draft must have exactly one line".into(),
            ));
        }

        let line = &t.lines[0];
        let side_str = match line.shop_side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        };
        let (received, paid) = match line.shop_side {
            Side::Buy => ("0.00".to_owned(), t.payment.amount_thb.to_string()),
            Side::Sell => (t.payment.amount_thb.to_string(), "0.00".to_owned()),
        };

        sqlx::query(
            "INSERT INTO transactions ( \
                id, organization_id, branch_id, mode, status, document_ref, quote_id, \
                booking_id, shift_id, submission_id, payload_hash, customer_id, customer_name, \
                nationality, type, code, amount, unit_size, rate, reference_rate, principal, \
                fee, bank_fee, received, paid, method, payment_ref, account, staff_id, \
                booking_no, notes, value_date, paid_at, recorded_at, recorded_by, \
                kiosk_request_id, staff_draft_id, count_confirmed \
            ) VALUES ( \
                $1, $2, $3, $4, 'DRAFT', NULL, $5, \
                $6, $7, $8, $9, $10, $11, \
                $12, $13, $14, $15::numeric, $16::numeric, $17::numeric, $18::numeric, $19::numeric, \
                $20::numeric, $21::numeric, $22::numeric, $23::numeric, $24, $25, $26, $27, \
                $28, $29, $30, $31, $32, $33, \
                $34, $35, $36 \
            )",
        )
        .bind(t.id.0)
        .bind(t.organization_id)
        .bind(t.branch_id.as_str())
        .bind(t.mode.as_str())
        .bind(t.quote_id)
        .bind(t.booking_id.as_deref())
        .bind(t.shift_id.as_deref())
        .bind(&t.submission_id)
        .bind(&t.payload_hash)
        .bind(&t.customer_id)
        .bind(&t.customer_name)
        .bind(&t.nationality)
        .bind(side_str)
        .bind(line.currency.as_str())
        .bind(line.quantity.value().to_string())
        .bind(line.unit_size.value().to_string())
        .bind(line.rate_snapshot.value().to_string())
        .bind(line.reference_rate.map(|r| r.value().to_string()))
        .bind(line.thb_amount.to_string())
        .bind(line.fee_thb.to_string())
        .bind(t.payment.bank_fee_thb.to_string())
        .bind(received)
        .bind(paid)
        .bind(t.payment.method.as_str())
        .bind(&t.payment.reference)
        .bind(&t.payment.account)
        .bind(&t.staff_id)
        .bind(&t.booking_no)
        .bind(&t.notes)
        .bind(t.value_date)
        .bind(t.payment.paid_at)
        .bind(t.recorded_at)
        .bind(&t.recorded_by)
        .bind(t.kiosk_request_id.as_deref())
        .bind(t.staff_draft_id.as_deref())
        .bind(t.count_confirmed)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        sqlx::query(
            "INSERT INTO transaction_lines ( \
                id, organization_id, transaction_id, quote_line_id, shop_side, currency, \
                denomination_id, quantity, unit_size, rate_snapshot, reference_rate, \
                thb_amount, fee_thb, settle_thb, cost_basis_thb, realized_margin_thb, \
                line_no, created_at, updated_at \
            ) VALUES ( \
                $1, $2, $3, $4, $5, $6, \
                $7, $8::numeric, $9::numeric, $10::numeric, $11::numeric, \
                $12::numeric, $13::numeric, $14::numeric, $15::numeric, $16::numeric, \
                $17, $18, $19 \
            )",
        )
        .bind(line.id)
        .bind(t.organization_id)
        .bind(t.id.0)
        .bind(line.quote_line_id)
        .bind(side_str)
        .bind(line.currency.as_str())
        .bind(None::<Uuid>)
        .bind(line.quantity.value().to_string())
        .bind(line.unit_size.value().to_string())
        .bind(line.rate_snapshot.value().to_string())
        .bind(line.reference_rate.map(|r| r.value().to_string()))
        .bind(line.thb_amount.to_string())
        .bind(line.fee_thb.to_string())
        .bind(line.settle_thb.to_string())
        .bind(line.cost_basis_thb.map(|m| m.to_string()))
        .bind(line.realized_margin_thb.map(|m| m.to_string()))
        .bind(line.line_no)
        .bind(t.recorded_at)
        .bind(t.recorded_at)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        sqlx::query(
            "INSERT INTO transaction_payments ( \
                id, organization_id, transaction_id, payment_no, method, reference, \
                account, slip_file_key, amount_thb, bank_fee_thb, paid_at, \
                created_at, updated_at \
            ) VALUES ( \
                $1, $2, $3, $4, $5, $6, \
                $7, $8, $9::numeric, $10::numeric, $11, \
                $12, $13 \
            )",
        )
        .bind(t.payment.id)
        .bind(t.organization_id)
        .bind(t.id.0)
        .bind(t.payment.payment_no)
        .bind(t.payment.method.as_str())
        .bind(&t.payment.reference)
        .bind(&t.payment.account)
        .bind(t.payment.slip_file_key.as_deref())
        .bind(t.payment.amount_thb.to_string())
        .bind(t.payment.bank_fee_thb.to_string())
        .bind(t.payment.paid_at)
        .bind(t.recorded_at)
        .bind(t.recorded_at)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        sqlx::query(
            "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, 'PASSPORT', $2)",
        )
        .bind(t.id.0)
        .bind(&t.passport_file_key)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        if let Some(slip) = &t.payment.slip_file_key {
            sqlx::query(
                "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, 'SLIP', $2)",
            )
            .bind(t.id.0)
            .bind(slip)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;
        }

        Ok(())
    }

    async fn find_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
    ) -> AppResult<Option<ExchangeTransaction>> {
        let row = sqlx::query(
            "SELECT \
                t.id, t.organization_id, t.branch_id, t.mode, t.status, t.document_ref, \
                t.quote_id, t.booking_id, t.shift_id, t.submission_id, t.payload_hash, \
                t.customer_id, t.customer_name, t.nationality, t.staff_id, t.booking_no, \
                t.notes, t.value_date, t.recorded_at, t.recorded_by, \
                t.completed_at, t.cancelled_at, t.cancel_reason, t.approval_request_id, \
                t.kiosk_request_id, t.staff_draft_id, t.count_confirmed, \
                (SELECT file_key FROM transaction_files f WHERE f.transaction_id = t.id AND f.kind = 'PASSPORT') AS passport_key \
            FROM transactions t \
            WHERE t.id = $1",
        )
        .bind(id.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let mode_str: String = row.try_get("mode").map_err(AppError::Database)?;
        let mode = if mode_str == "DEMO" {
            Mode::Demo
        } else {
            Mode::Live
        };

        let status_str: String = row.try_get("status").map_err(AppError::Database)?;
        let status = TransactionStatus::parse(&status_str)
            .ok_or_else(|| AppError::Internal(format!("unknown status: {status_str}")))?;

        let passport_file_key: Option<String> =
            row.try_get("passport_key").map_err(AppError::Database)?;
        let passport_file_key = passport_file_key.unwrap_or_default();

        // Load lines
        let line_rows = sqlx::query(
            "SELECT \
                id, quote_line_id, shop_side, currency, quantity::text, unit_size::text, \
                rate_snapshot::text, reference_rate::text, thb_amount::text, fee_thb::text, \
                settle_thb::text, cost_basis_thb::text, realized_margin_thb::text, line_no \
            FROM transaction_lines \
            WHERE transaction_id = $1 \
            ORDER BY line_no",
        )
        .bind(id.0)
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let mut lines = Vec::with_capacity(line_rows.len());
        for lr in line_rows {
            let side_str: String = lr.try_get("shop_side").map_err(AppError::Database)?;
            let side = if side_str == "SELL" {
                Side::Sell
            } else {
                Side::Buy
            };
            let curr_str: String = lr.try_get("currency").map_err(AppError::Database)?;
            let currency = CurrencyCode::parse(curr_str.trim())
                .map_err(|e| AppError::Internal(format!("currency: {e}")))?;
            let qty_str: String = lr.try_get("quantity").map_err(AppError::Database)?;
            let quantity = ForeignAmount::from_decimal(
                plain_decimal(&qty_str)
                    .ok_or_else(|| AppError::Internal("quantity decimal".into()))?,
            );
            let unit_str: String = lr.try_get("unit_size").map_err(AppError::Database)?;
            let unit_size = UnitSize::parse(&unit_str)
                .map_err(|e| AppError::Internal(format!("unit_size: {e}")))?;
            let rate_str: String = lr.try_get("rate_snapshot").map_err(AppError::Database)?;
            let rate_snapshot =
                Rate::parse(&rate_str).map_err(|e| AppError::Internal(format!("rate: {e}")))?;
            let ref_rate_opt: Option<String> =
                lr.try_get("reference_rate").map_err(AppError::Database)?;
            let reference_rate = ref_rate_opt
                .map(|s| {
                    Rate::parse(&s).map_err(|e| AppError::Internal(format!("reference_rate: {e}")))
                })
                .transpose()?;
            let thb_str: String = lr.try_get("thb_amount").map_err(AppError::Database)?;
            let thb_amount = Money::parse(&thb_str)
                .map_err(|e| AppError::Internal(format!("thb_amount: {e}")))?;
            let fee_str: String = lr.try_get("fee_thb").map_err(AppError::Database)?;
            let fee_thb =
                Money::parse(&fee_str).map_err(|e| AppError::Internal(format!("fee_thb: {e}")))?;
            let settle_str: String = lr.try_get("settle_thb").map_err(AppError::Database)?;
            let settle_thb = Money::parse(&settle_str)
                .map_err(|e| AppError::Internal(format!("settle_thb: {e}")))?;
            let cost_opt: Option<String> =
                lr.try_get("cost_basis_thb").map_err(AppError::Database)?;
            let cost_basis_thb = cost_opt
                .map(|s| {
                    Money::parse(&s).map_err(|e| AppError::Internal(format!("cost_basis: {e}")))
                })
                .transpose()?;
            let margin_opt: Option<String> = lr
                .try_get("realized_margin_thb")
                .map_err(AppError::Database)?;
            let realized_margin_thb = margin_opt
                .map(|s| {
                    Money::parse(&s)
                        .map_err(|e| AppError::Internal(format!("realized_margin: {e}")))
                })
                .transpose()?;

            lines.push(TransactionLine {
                id: lr.try_get("id").map_err(AppError::Database)?,
                quote_line_id: lr.try_get("quote_line_id").map_err(AppError::Database)?,
                shop_side: side,
                currency,
                quantity,
                unit_size,
                rate_snapshot,
                reference_rate,
                thb_amount,
                fee_thb,
                settle_thb,
                cost_basis_thb,
                realized_margin_thb,
                line_no: lr.try_get("line_no").map_err(AppError::Database)?,
            });
        }

        // Load payment
        let p_row = sqlx::query(
            "SELECT \
                id, payment_no, method, reference, account, slip_file_key, \
                amount_thb::text, bank_fee_thb::text, paid_at \
            FROM transaction_payments \
            WHERE transaction_id = $1 \
            ORDER BY payment_no \
            LIMIT 1",
        )
        .bind(id.0)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let p_method_str: String = p_row.try_get("method").map_err(AppError::Database)?;
        let p_method = parse_payment_method(&p_method_str)?;
        let p_amount_str: String = p_row.try_get("amount_thb").map_err(AppError::Database)?;
        let p_amount = Money::parse(&p_amount_str)
            .map_err(|e| AppError::Internal(format!("payment amount: {e}")))?;
        let p_fee_str: String = p_row.try_get("bank_fee_thb").map_err(AppError::Database)?;
        let p_fee =
            Money::parse(&p_fee_str).map_err(|e| AppError::Internal(format!("bank fee: {e}")))?;

        let payment = TransactionPayment {
            id: p_row.try_get("id").map_err(AppError::Database)?,
            payment_no: p_row.try_get("payment_no").map_err(AppError::Database)?,
            method: p_method,
            reference: p_row.try_get("reference").map_err(AppError::Database)?,
            account: p_row.try_get("account").map_err(AppError::Database)?,
            slip_file_key: p_row.try_get("slip_file_key").map_err(AppError::Database)?,
            amount_thb: p_amount,
            bank_fee_thb: p_fee,
            paid_at: p_row.try_get("paid_at").map_err(AppError::Database)?,
        };

        // Load receipts
        let r_rows = sqlx::query(
            "SELECT id, number, revision, issued_at, document_ref \
            FROM receipts \
            WHERE transaction_id = $1 \
            ORDER BY revision",
        )
        .bind(id.0)
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let mut receipts = Vec::with_capacity(r_rows.len());
        for rr in r_rows {
            receipts.push(Receipt {
                id: rr.try_get("id").map_err(AppError::Database)?,
                number: rr.try_get("number").map_err(AppError::Database)?,
                revision: rr.try_get("revision").map_err(AppError::Database)?,
                issued_at: rr.try_get("issued_at").map_err(AppError::Database)?,
                document_ref: rr.try_get("document_ref").map_err(AppError::Database)?,
            });
        }

        let branch_str: String = row.try_get("branch_id").map_err(AppError::Database)?;

        Ok(Some(ExchangeTransaction {
            id,
            organization_id: row.try_get("organization_id").map_err(AppError::Database)?,
            branch_id: BranchId::new(branch_str),
            mode,
            status,
            document_ref: row.try_get("document_ref").map_err(AppError::Database)?,
            quote_id: row.try_get("quote_id").map_err(AppError::Database)?,
            booking_id: row.try_get("booking_id").map_err(AppError::Database)?,
            shift_id: row.try_get("shift_id").map_err(AppError::Database)?,
            submission_id: row.try_get("submission_id").map_err(AppError::Database)?,
            payload_hash: row.try_get("payload_hash").map_err(AppError::Database)?,
            customer_id: row.try_get("customer_id").map_err(AppError::Database)?,
            customer_name: row.try_get("customer_name").map_err(AppError::Database)?,
            nationality: row.try_get("nationality").map_err(AppError::Database)?,
            staff_id: row.try_get("staff_id").map_err(AppError::Database)?,
            booking_no: row.try_get("booking_no").map_err(AppError::Database)?,
            notes: row.try_get("notes").map_err(AppError::Database)?,
            value_date: row.try_get("value_date").map_err(AppError::Database)?,
            recorded_at: row.try_get("recorded_at").map_err(AppError::Database)?,
            recorded_by: row.try_get("recorded_by").map_err(AppError::Database)?,
            completed_at: row.try_get("completed_at").map_err(AppError::Database)?,
            cancelled_at: row.try_get("cancelled_at").map_err(AppError::Database)?,
            cancel_reason: row.try_get("cancel_reason").map_err(AppError::Database)?,
            approval_request_id: row
                .try_get("approval_request_id")
                .map_err(AppError::Database)?,
            passport_file_key,
            kiosk_request_id: row
                .try_get("kiosk_request_id")
                .map_err(AppError::Database)?,
            staff_draft_id: row.try_get("staff_draft_id").map_err(AppError::Database)?,
            count_confirmed: row.try_get("count_confirmed").map_err(AppError::Database)?,
            lines,
            payment,
            receipts,
        }))
    }

    async fn set_status_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        r#move: StatusMove,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        if r#move.to == TransactionStatus::Completed {
            return Err(AppError::Domain(
                "validationFailed: complete_in must be used to complete".into(),
            ));
        }
        if r#move.to == TransactionStatus::Cancelled
            && r#move
                .reason
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(AppError::Domain(
                "validationFailed: cancel reason is required".into(),
            ));
        }

        let row = sqlx::query("SELECT status FROM transactions WHERE id = $1 FOR UPDATE")
            .bind(id.0)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;

        let Some(row) = row else {
            return Err(AppError::Domain(
                "staleVersion: transaction not found".into(),
            ));
        };

        let current_status: String = row.try_get("status").map_err(AppError::Database)?;
        if current_status != r#move.from.as_str() {
            return Err(AppError::Domain(format!(
                "staleVersion: current status {current_status} != expected {}",
                r#move.from.as_str()
            )));
        }

        match r#move.to {
            TransactionStatus::Cancelled => {
                sqlx::query(
                    "UPDATE transactions \
                     SET status = 'CANCELLED', cancelled_at = $2, cancel_reason = $3 \
                     WHERE id = $1",
                )
                .bind(id.0)
                .bind(now)
                .bind(r#move.reason)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            }
            TransactionStatus::PendingApproval => {
                sqlx::query(
                    "UPDATE transactions \
                     SET status = 'PENDING_APPROVAL', approval_request_id = $2 \
                     WHERE id = $1",
                )
                .bind(id.0)
                .bind(r#move.approval_request_id)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            }
            _ => {
                sqlx::query("UPDATE transactions SET status = $2 WHERE id = $1")
                    .bind(id.0)
                    .bind(r#move.to.as_str())
                    .execute(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
            }
        }

        Ok(())
    }

    async fn complete_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        receipt_file_key: &'a str,
        now: DateTime<Utc>,
    ) -> AppResult<(String, Receipt)> {
        let row = sqlx::query(
            "SELECT status, organization_id, mode FROM transactions WHERE id = $1 FOR UPDATE",
        )
        .bind(id.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let Some(row) = row else {
            return Err(AppError::Domain(
                "staleVersion: transaction not found".into(),
            ));
        };

        let status: String = row.try_get("status").map_err(AppError::Database)?;
        if status != "CONFIRMED" {
            return Err(AppError::Domain(
                "staleVersion: transaction is not CONFIRMED".into(),
            ));
        }

        let organization_id: Uuid = row.try_get("organization_id").map_err(AppError::Database)?;
        let mode_str: String = row.try_get("mode").map_err(AppError::Database)?;
        let mode = if mode_str == "DEMO" {
            Mode::Demo
        } else {
            Mode::Live
        };

        let counter_key = DocumentRef::counter_key(organization_id, mode, now);
        let number: i64 = sqlx::query_scalar(
            "INSERT INTO counters (key, value) VALUES ($1, 1) \
             ON CONFLICT (key) DO UPDATE SET value = counters.value + 1 \
             RETURNING value",
        )
        .bind(&counter_key)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let document_ref = DocumentRef::format(mode, now, number);

        sqlx::query(
            "UPDATE transactions \
             SET status = 'COMPLETED', document_ref = $2, completed_at = $3 \
             WHERE id = $1",
        )
        .bind(id.0)
        .bind(&document_ref)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let receipt_id = Uuid::new_v4();
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
        .bind(organization_id)
        .bind(id.0)
        .bind(&document_ref)
        .bind(now)
        .bind(receipt_file_key)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let receipt = Receipt {
            id: receipt_id,
            number: document_ref.clone(),
            revision: 1,
            issued_at: now,
            document_ref: receipt_file_key.to_string(),
        };

        Ok((document_ref, receipt))
    }
}
