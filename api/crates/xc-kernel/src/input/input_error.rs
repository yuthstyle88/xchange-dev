//! STUB (ticket #88).

/// A refusal of an input rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    Text,
    DateFormat,
    Date,
    Number,
    TooManyDecimals,
}

impl InputError {
    /// The sentence v2 says for this refusal.
    pub fn sentence(self) -> &'static str {
        match self {
            Self::Text => "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด",
            Self::DateFormat => "รูปแบบวันที่ไม่ถูกต้อง",
            Self::Date => "วันที่ไม่ถูกต้อง",
            Self::Number => "จำนวนเงินหรือเรทไม่ถูกต้อง",
            Self::TooManyDecimals => "ทศนิยมเกินกำหนด",
        }
    }
}

impl From<InputError> for pos_error::AppError {
    fn from(e: InputError) -> Self {
        pos_error::AppError::Domain(e.sentence().to_owned())
    }
}
