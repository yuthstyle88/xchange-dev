//! Cash domain errors.

use xc_auth::domain::access::Refusal;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CashError {
    #[error("ผู้ตรวจนับและผู้รับรองต้องไม่เป็นบุคคลเดียวกัน")]
    SameCounter,
    #[error("ผู้รับรองต้องเป็นผู้รับมอบหมาย")]
    NotReceiver,
    #[error("ผู้ใช้ไม่ได้เป็นเจ้าของกะนี้")]
    NotOwnShift,
    #[error("สามารถรับรองได้เฉพาะการนับล่าสุดเท่านั้น")]
    NotNewestCount,
    #[error("ยอดเปลี่ยนระหว่างตรวจนับ กรุณารีเฟรชและตรวจสอบอีกครั้ง")]
    BalancesChanged,
    #[error("จำนวนเงินมีทศนิยมเกินกำหนด")]
    TooManyDecimals,
    #[error("มีกะเปิดอยู่แล้ว กรุณารีเฟรช")]
    ShiftAlreadyOpen,
    #[error("{0:?}")]
    Refused(Refusal),
    #[error("รหัสคำขอนี้ถูกใช้โดยผู้ใช้อื่นแล้ว")]
    ReplayOfAnotherUser,
    #[error("รอฝ่ายการเงินอนุมัติ: {0}")]
    AwaitingFinance(String),
    #[error("ฝ่ายการเงินปฏิเสธ: {0}")]
    RejectedByFinance(String),
}

impl CashError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::SameCounter => "sameCounter",
            Self::NotReceiver => "notReceiver",
            Self::NotOwnShift => "notOwnShift",
            Self::NotNewestCount => "notNewestCount",
            Self::BalancesChanged => "balancesChanged",
            Self::TooManyDecimals => "tooManyDecimals",
            Self::ShiftAlreadyOpen => "shiftAlreadyOpen",
            Self::ReplayOfAnotherUser => "replayOfAnotherUser",
            Self::Refused(_) => "refused",
            Self::AwaitingFinance(_) => "awaitingFinance",
            Self::RejectedByFinance(_) => "rejectedByFinance",
        }
    }
}

impl From<CashError> for pos_error::AppError {
    fn from(e: CashError) -> Self {
        pos_error::AppError::Domain(e.to_string())
    }
}
