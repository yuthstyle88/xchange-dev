//! PaymentMethod value object.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaymentMethod {
    Cash,
    ScbTransfer,
    BankTransfer,
    CreditCard,
    Alipay,
    PromptPay,
}

impl PaymentMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cash => "Cash",
            Self::ScbTransfer => "SCB Transfer",
            Self::BankTransfer => "Bank Transfer",
            Self::CreditCard => "Credit Card",
            Self::Alipay => "Alipay",
            Self::PromptPay => "PromptPay",
        }
    }

    pub fn is_cash(self) -> bool {
        matches!(self, Self::Cash)
    }
}
