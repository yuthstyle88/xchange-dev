//! Transaction row DTO for v3 (§7, §12.2).

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_kernel::money::{ExchangeArithmetic, ForeignAmount, Rate, UnitSize, fixed6};

/// Select list for transaction list views (§7, §8).
pub const ROW_COLUMNS: &str = "\
    t.id, \
    t.status, \
    t.cancel_reason, \
    t.document_ref, \
    t.value_date, \
    t.recorded_at, \
    l.shop_side AS type, \
    l.currency::text AS code, \
    l.quantity::text AS amount, \
    l.rate_snapshot::text AS rate, \
    l.reference_rate::text AS reference_rate, \
    l.unit_size::text AS unit_size, \
    l.fee_thb::text AS fee, \
    p.bank_fee_thb::text AS bank_fee, \
    l.thb_amount::text AS principal, \
    l.settle_thb::text AS settlement, \
    p.method, \
    p.reference AS payment_ref, \
    t.customer_id, \
    t.customer_name, \
    t.staff_id, \
    t.notes, \
    t.shift_id, \
    p.slip_file_key AS slip_key";

/// From clause with joins for transaction list views (§7, §8).
pub const ROW_FROM: &str = "\
    transactions t \
    JOIN transaction_lines l ON l.transaction_id = t.id AND l.line_no = 1 \
    JOIN transaction_payments p ON p.transaction_id = t.id AND p.payment_no = 1";

/// An exchange record as stored in the database / domain.
#[derive(Debug, Clone)]
pub struct TransactionRecord {
    pub document_ref: String,
    pub recorded_at: DateTime<Utc>,
    pub value_date: NaiveDate,
    pub side: String,
    pub currency: String,
    pub amount: Decimal,
    pub rate: Rate,
    pub reference_rate: Option<Rate>,
    pub unit_size: UnitSize,
    pub fee: Decimal,
    pub bank_fee: Decimal,
    pub method: String,
    pub payment_ref: String,
    pub customer_id: String,
    pub customer_name: String,
    pub staff: String,
    pub notes: String,
    pub shift_id: Option<String>,
    pub slip_key: Option<String>,
}

/// SQL row mapped from the `transactions` table (with joined slip key).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TxSqlRow {
    pub id: Uuid,
    pub status: String,
    pub cancel_reason: Option<String>,
    pub document_ref: Option<String>,
    pub value_date: NaiveDate,
    pub recorded_at: DateTime<Utc>,
    pub r#type: String,
    pub code: String,
    pub amount: String,
    pub rate: String,
    pub reference_rate: Option<String>,
    pub unit_size: String,
    pub fee: String,
    pub bank_fee: String,
    pub principal: String,
    pub settlement: String,
    pub method: String,
    pub payment_ref: String,
    pub customer_id: String,
    pub customer_name: String,
    pub staff_id: String,
    pub notes: String,
    pub shift_id: Option<String>,
    pub slip_key: Option<String>,
}

pub type TransactionSqlRow = TxSqlRow;

impl TxSqlRow {
    /// Fallibly decodes into a [`TransactionRecord`], returning an error if any money column fails to parse.
    pub fn try_into_record(self) -> Result<TransactionRecord, AppError> {
        TransactionRecord::try_from(self)
    }
}

impl TryFrom<TxSqlRow> for TransactionRecord {
    type Error = AppError;

    fn try_from(r: TxSqlRow) -> Result<Self, Self::Error> {
        let doc_ref = r.document_ref.clone().unwrap_or_else(|| r.id.to_string());
        let amount = r.amount.parse::<Decimal>().map_err(|e| {
            AppError::Internal(format!(
                "invalid amount {:?} in transaction {}: {e}",
                r.amount, doc_ref
            ))
        })?;
        let rate = Rate::parse(r.rate.trim()).map_err(|e| {
            AppError::Internal(format!(
                "invalid rate {:?} in transaction {}: {e}",
                r.rate, doc_ref
            ))
        })?;
        let reference_rate = match r.reference_rate {
            Some(ref s) if !s.trim().is_empty() => Some(Rate::parse(s.trim()).map_err(|e| {
                AppError::Internal(format!(
                    "invalid reference_rate {:?} in transaction {}: {e}",
                    s, doc_ref
                ))
            })?),
            _ => None,
        };
        let unit_size = UnitSize::parse(r.unit_size.trim()).map_err(|e| {
            AppError::Internal(format!(
                "invalid unit_size {:?} in transaction {}: {e}",
                r.unit_size, doc_ref
            ))
        })?;
        let fee = r.fee.parse::<Decimal>().map_err(|e| {
            AppError::Internal(format!(
                "invalid fee {:?} in transaction {}: {e}",
                r.fee, doc_ref
            ))
        })?;
        let bank_fee = r.bank_fee.parse::<Decimal>().map_err(|e| {
            AppError::Internal(format!(
                "invalid bank_fee {:?} in transaction {}: {e}",
                r.bank_fee, doc_ref
            ))
        })?;

        Ok(Self {
            document_ref: doc_ref,
            recorded_at: r.recorded_at,
            value_date: r.value_date,
            side: r.r#type,
            currency: r.code.trim().to_string(),
            amount,
            rate,
            reference_rate,
            unit_size,
            fee,
            bank_fee,
            method: r.method,
            payment_ref: r.payment_ref,
            customer_id: r.customer_id,
            customer_name: r.customer_name,
            staff: r.staff_id,
            notes: r.notes,
            shift_id: r.shift_id,
            slip_key: r.slip_key,
        })
    }
}

/// Shared transaction row in v3 admin views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction_row.ts")]
pub struct TransactionRow {
    pub id: String,
    pub date: String,
    pub created: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub r#type: String,
    pub currency: String,
    pub amount: String,
    pub rate: String,
    pub unit_size: String,
    pub principal: String,
    pub fee: String,
    pub bank_fee: String,
    pub settlement: String,
    pub method: String,
    pub customer: String,
    pub customer_id: String,
    pub staff: String,
    pub status: String,
    pub notes: Option<String>,
    pub cancel_reason: Option<String>,
    pub shift_id: Option<String>,
    pub reference_rate: Option<String>,
    pub reference_value: Option<String>,
    pub reference_spread: Option<String>,
    pub reference_percent: Option<String>,
    pub payment_ref: Option<String>,
    pub slip_url: Option<String>,
}

impl TransactionRow {
    /// Builds a row from a saved exchange record (design §12.2, W7).
    pub fn from_record(record: &TransactionRecord) -> Self {
        Self::try_from_record(record)
            .expect("transaction figure is not representable at 2 decimal places")
    }

    /// Builds a row from a saved exchange record, answering `AppError::Internal` if a computed THB
    /// figure cannot be represented at 2 decimal places (#2063 / #3064).
    pub fn try_from_record(record: &TransactionRecord) -> Result<Self, AppError> {
        let is_buy = record.side == "BUY";
        let overflow = |what: &str| {
            AppError::Internal(format!(
                "{what} of transaction {} is not representable at 2 decimal places",
                record.document_ref
            ))
        };

        // principal = amount * rate / unit_size, half up to satang (2 decimals) through ExchangeArithmetic::try_thb
        let foreign = ForeignAmount::from_decimal(record.amount);
        let principal = Decimal::from_i128_with_scale(
            ExchangeArithmetic::try_thb(foreign, record.rate, record.unit_size)
                .map_err(|_| overflow("principal"))?
                .minor_i128(),
            2,
        );

        // settlement = principal - fee (BUY), principal + fee (SELL)
        let settlement = if is_buy {
            principal - record.fee
        } else {
            principal + record.fee
        };

        // reference fields
        let (ref_rate_str, ref_val_str, ref_spread_str, ref_pct_str) =
            if let Some(ref_rate) = record.reference_rate {
                let ref_val = Decimal::from_i128_with_scale(
                    ExchangeArithmetic::try_thb(foreign, ref_rate, record.unit_size)
                        .map_err(|_| overflow("reference value"))?
                        .minor_i128(),
                    2,
                );

                let spread = if is_buy {
                    ref_val - principal
                } else {
                    principal - ref_val
                };

                let pct_str = if spread.is_zero() || ref_val.is_zero() {
                    "0.000000".to_string()
                } else {
                    let raw_pct = (spread / ref_val) * Decimal::from(100);
                    fixed6(raw_pct)
                };

                let spread_str = if spread.is_zero() {
                    "0.00".to_string()
                } else {
                    format!("{:.2}", spread)
                };

                (
                    Some(format!("{:.8}", ref_rate.value())),
                    Some(format!("{:.2}", ref_val)),
                    Some(spread_str),
                    Some(pct_str),
                )
            } else {
                (None, None, None, None)
            };

        let notes = if record.notes.is_empty() {
            None
        } else {
            Some(record.notes.clone())
        };

        let payment_ref = if record.payment_ref.is_empty() {
            None
        } else {
            Some(record.payment_ref.clone())
        };

        let method = if record.method == "SCB Transfer" {
            "Bank Transfer".to_string()
        } else {
            record.method.clone()
        };

        let slip_url = record.slip_key.as_ref().map(|k| format!("/files/{k}"));

        Ok(Self {
            id: record.document_ref.clone(),
            date: record.value_date.format("%Y-%m-%d").to_string(),
            created: record
                .recorded_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            r#type: record.side.clone(),
            currency: record.currency.trim().to_string(),
            amount: format!("{:.2}", record.amount),
            rate: format!("{:.8}", record.rate.value()),
            unit_size: record.unit_size.value().normalize().to_string(),
            principal: format!("{:.2}", principal),
            fee: format!("{:.2}", record.fee),
            bank_fee: format!("{:.2}", record.bank_fee),
            settlement: format!("{:.2}", settlement),
            method,
            customer: record.customer_name.clone(),
            customer_id: record.customer_id.clone(),
            staff: record.staff.clone(),
            status: "COMPLETED".to_string(),
            notes,
            cancel_reason: None,
            shift_id: record.shift_id.clone(),
            reference_rate: ref_rate_str,
            reference_value: ref_val_str,
            reference_spread: ref_spread_str,
            reference_percent: ref_pct_str,
            payment_ref,
            slip_url,
        })
    }

    /// Builds a row from a SQL row and child lines/payments.
    pub fn from_sql_row(r: TxSqlRow) -> Result<Self, AppError> {
        let status = r.status.clone();
        let cancel_reason = r.cancel_reason.clone();
        let principal_str = r.principal.clone();
        let settlement_str = r.settlement.clone();
        let record = TransactionRecord::try_from(r)?;
        let mut row = Self::try_from_record(&record)?;
        row.status = status;
        row.cancel_reason = cancel_reason;
        if let Ok(p) = principal_str.parse::<Decimal>() {
            row.principal = format!("{:.2}", p);
        }
        if let Ok(s) = settlement_str.parse::<Decimal>() {
            row.settlement = format!("{:.2}", s);
        }
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sql_row() -> TxSqlRow {
        TxSqlRow {
            id: Uuid::nil(),
            status: "COMPLETED".into(),
            cancel_reason: None,
            document_ref: Some("XC-TEST-001".into()),
            value_date: NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
            recorded_at: Utc::now(),
            r#type: "BUY".into(),
            code: "USD".into(),
            amount: "100.00".into(),
            rate: "32.1000".into(),
            reference_rate: Some("32.5000".into()),
            unit_size: "1".into(),
            fee: "0.00".into(),
            bank_fee: "0.00".into(),
            principal: "3210.00".into(),
            settlement: "3210.00".into(),
            method: "Cash".into(),
            payment_ref: "".into(),
            customer_id: "C-1".into(),
            customer_name: "Test Customer".into(),
            staff_id: "EMP-1".into(),
            notes: "".into(),
            shift_id: None,
            slip_key: None,
        }
    }

    #[test]
    fn bad_unit_size_fails_decode() {
        let mut row = sample_sql_row();
        row.unit_size = "broken_unit".into();
        let err = TransactionRecord::try_from(row).unwrap_err();
        match err {
            AppError::Internal(msg) => {
                assert!(msg.contains("invalid unit_size"), "{msg}");
                assert!(msg.contains("broken_unit"), "{msg}");
            }
            other => panic!("expected AppError::Internal, got {other:?}"),
        }
    }

    #[test]
    fn zero_unit_size_fails_decode() {
        let mut row = sample_sql_row();
        row.unit_size = "0".into();
        let err = TransactionRecord::try_from(row).unwrap_err();
        match err {
            AppError::Internal(msg) => assert!(msg.contains("invalid unit_size"), "{msg}"),
            other => panic!("expected AppError::Internal, got {other:?}"),
        }
    }

    #[test]
    fn bad_amount_fails_decode_rather_than_zero() {
        let mut row = sample_sql_row();
        row.amount = "not_a_number".into();
        let res = TransactionRecord::try_from(row);
        assert!(res.is_err(), "corrupt amount must return Err");
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("invalid amount"), "{err_msg}");
    }

    #[test]
    fn bad_rate_fails_decode() {
        let mut row = sample_sql_row();
        row.rate = "invalid_rate".into();
        let err = TransactionRecord::try_from(row).unwrap_err();
        match err {
            AppError::Internal(msg) => assert!(msg.contains("invalid rate"), "{msg}"),
            other => panic!("expected AppError::Internal, got {other:?}"),
        }
    }

    #[test]
    fn bad_reference_rate_fails_decode() {
        let mut row = sample_sql_row();
        row.reference_rate = Some("invalid_ref_rate".into());
        let err = TransactionRecord::try_from(row).unwrap_err();
        match err {
            AppError::Internal(msg) => assert!(msg.contains("invalid reference_rate"), "{msg}"),
            other => panic!("expected AppError::Internal, got {other:?}"),
        }
    }

    #[test]
    fn bad_fee_fails_decode() {
        let mut row = sample_sql_row();
        row.fee = "broken_fee".into();
        assert!(TransactionRecord::try_from(row).is_err());
    }

    #[test]
    fn bad_bank_fee_fails_decode() {
        let mut row = sample_sql_row();
        row.bank_fee = "broken_bank_fee".into();
        assert!(TransactionRecord::try_from(row).is_err());
    }

    #[test]
    fn valid_sql_row_decodes_cleanly() {
        let row = sample_sql_row();
        let rec = TransactionRecord::try_from(row).expect("valid row decodes");
        assert_eq!(rec.amount, Decimal::from(100));
        assert_eq!(rec.rate, Rate::parse("32.1000").unwrap());
        assert_eq!(rec.unit_size, UnitSize::ONE);
    }
}
