//! SaveTransaction command.

use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveTransaction {
    pub customer: ReserveCustomer,
    pub submission_id: String,
    pub customer_id: String,
    pub side: String,
    pub currency: String,
    pub amount: String,
    pub rate: String,
    pub fee: String,
    pub bank_fee: String,
    pub method: String,
    pub payment_ref: String,
    pub account: String,
    pub staff: String,
    pub booking: String,
    pub notes: String,
    pub value_date: String,
    pub paid_at: String,
    pub payment_confirmed: bool,
    pub identity_confirmed: bool,
    pub passport: Option<String>,
    pub slip: Option<String>,
    pub reference_rate: Option<String>,
}
