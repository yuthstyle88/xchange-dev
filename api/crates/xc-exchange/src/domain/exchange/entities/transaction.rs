//! Transaction entity.

use chrono::{DateTime, NaiveDate, Utc};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Money, Rate, Side};

use crate::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_kernel::mode::Mode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub document_ref: String,
    pub branch_id: BranchId,
    pub mode: Mode,
    pub shift_id: Option<String>,
    pub submission_id: String,
    pub customer_id: String,
    pub customer_name: String,
    pub nationality: String,
    pub side: Side,
    pub currency: String,
    pub amount: Money,
    pub rate: Rate,
    pub reference_rate: Option<Rate>,
    pub principal: Money,
    pub fee: Money,
    pub bank_fee: Money,
    pub received: Money,
    pub paid: Money,
    pub method: PaymentMethod,
    pub payment_ref: String,
    pub account: String,
    pub staff: String,
    pub booking: String,
    pub notes: String,
    pub value_date: NaiveDate,
    pub paid_at: DateTime<Utc>,
    pub passport_key: String,
    pub slip_key: Option<String>,
    pub receipt_key: String,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: String,
}
