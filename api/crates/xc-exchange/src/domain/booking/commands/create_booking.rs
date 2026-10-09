//! Create booking command.

use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use xc_kernel::money::{CurrencyCode, Money, Side};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBooking {
    pub request_id: String,
    pub name: String,
    pub phone: String,
    pub side: Side,
    pub currency: CurrencyCode,
    pub amount: Money,
    pub expected_rate: Decimal,
    pub customer_id: Option<String>,
}

impl CreateBooking {
    pub fn request_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.request_id.as_bytes());
        hasher.update(b"|");
        hasher.update(self.name.as_bytes());
        hasher.update(b"|");
        hasher.update(self.phone.as_bytes());
        hasher.update(b"|");
        hasher.update(
            match self.side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            }
            .as_bytes(),
        );
        hasher.update(b"|");
        hasher.update(self.currency.as_str().as_bytes());
        hasher.update(b"|");
        hasher.update(self.amount.to_string().as_bytes());
        hasher.update(b"|");
        hasher.update(self.expected_rate.to_string().as_bytes());
        hasher.update(b"|");
        if let Some(ref cid) = self.customer_id {
            hasher.update(cid.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }
}
