//! Transaction row DTO for v3 (§7, §12.2).

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppError;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_kernel::money::{ExchangeArithmetic, ForeignAmount, Rate, UnitSize};

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
    pub document_ref: String,
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
        let amount = r.amount.parse::<Decimal>().map_err(|e| {
            AppError::Internal(format!(
                "invalid amount {:?} in transaction {}: {e}",
                r.amount, r.document_ref
            ))
        })?;
        let rate = Rate::parse(r.rate.trim()).map_err(|e| {
            AppError::Internal(format!(
                "invalid rate {:?} in transaction {}: {e}",
                r.rate, r.document_ref
            ))
        })?;
        let reference_rate = match r.reference_rate {
            Some(ref s) if !s.trim().is_empty() => Some(Rate::parse(s.trim()).map_err(|e| {
                AppError::Internal(format!(
                    "invalid reference_rate {:?} in transaction {}: {e}",
                    s, r.document_ref
                ))
            })?),
            _ => None,
        };
        let unit_size = UnitSize::parse(r.unit_size.trim()).map_err(|e| {
            AppError::Internal(format!(
                "invalid unit_size {:?} in transaction {}: {e}",
                r.unit_size, r.document_ref
            ))
        })?;
        let fee = r.fee.parse::<Decimal>().map_err(|e| {
            AppError::Internal(format!(
                "invalid fee {:?} in transaction {}: {e}",
                r.fee, r.document_ref
            ))
        })?;
        let bank_fee = r.bank_fee.parse::<Decimal>().map_err(|e| {
            AppError::Internal(format!(
                "invalid bank_fee {:?} in transaction {}: {e}",
                r.bank_fee, r.document_ref
            ))
        })?;

        Ok(Self {
            document_ref: r.document_ref,
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
        let is_buy = record.side == "BUY";

        // principal = amount * rate / unit_size, half up to satang (2 decimals) through ExchangeArithmetic::thb
        let foreign = ForeignAmount::from_decimal(record.amount);
        let principal = Decimal::from_i128_with_scale(
            ExchangeArithmetic::thb(foreign, record.rate, record.unit_size).minor_i128(),
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
                    ExchangeArithmetic::thb(foreign, ref_rate, record.unit_size).minor_i128(),
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
                    let rounded_pct =
                        raw_pct.round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero);
                    format!("{:.6}", rounded_pct)
                };

                let spread_str = if spread.is_zero() {
                    "0.00".to_string()
                } else {
                    format!("{:.2}", spread)
                };

                (
                    Some(format!("{:.4}", ref_rate.value())),
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

        Self {
            id: record.document_ref.clone(),
            date: record.value_date.format("%Y-%m-%d").to_string(),
            created: record
                .recorded_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            r#type: record.side.clone(),
            currency: record.currency.trim().to_string(),
            amount: format!("{:.2}", record.amount),
            rate: format!("{:.4}", record.rate.value()),
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sql_row() -> TxSqlRow {
        TxSqlRow {
            document_ref: "XC-TEST-001".into(),
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

