#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmountIn {
    Foreign,
    Thb,
}

impl AmountIn {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "FOREIGN" => Some(Self::Foreign),
            "THB" => Some(Self::Thb),
            _ => None,
        }
    }
}
