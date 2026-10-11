//! TransactionPayment entity.

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_kernel::money::Money;

use crate::domain::exchange::value_objects::payment_method::PaymentMethod;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionPayment {
    pub id: Uuid,
    pub payment_no: i32,
    pub method: PaymentMethod,
    pub reference: String,
    pub account: String,
    pub slip_file_key: Option<String>,
    pub amount_thb: Money,
    pub bank_fee_thb: Money,
    pub paid_at: DateTime<Utc>,
}
