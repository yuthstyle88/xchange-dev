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

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "Cash" => Some(Self::Cash),
            "SCB Transfer" => Some(Self::ScbTransfer),
            "Bank Transfer" => Some(Self::BankTransfer),
            "Credit Card" => Some(Self::CreditCard),
            "Alipay" => Some(Self::Alipay),
            "PromptPay" => Some(Self::PromptPay),
            _ => None,
        }
    }
}
