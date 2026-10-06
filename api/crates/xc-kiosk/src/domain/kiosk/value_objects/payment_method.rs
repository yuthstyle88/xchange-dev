#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentMethod {
    Cash,
    CreditCard,
    Alipay,
    PromptPay,
    BankTransfer,
}

impl PaymentMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cash => "Cash",
            Self::CreditCard => "Credit Card",
            Self::Alipay => "Alipay",
            Self::PromptPay => "PromptPay",
            Self::BankTransfer => "Bank Transfer",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "Cash" => Some(Self::Cash),
            "Credit Card" => Some(Self::CreditCard),
            "Alipay" => Some(Self::Alipay),
            "PromptPay" => Some(Self::PromptPay),
            "Bank Transfer" => Some(Self::BankTransfer),
            _ => None,
        }
    }
}
