//! Refusals of the kiosk service that the pure rules do not know.

/// `Display` is the sentence the screen is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KioskServiceError {
    #[error("ไม่พบสาขาที่เลือก กรุณารีเฟรช")]
    NoSuchBranch,
    #[error("Invalid receipt image.")]
    InvalidReceiptImage,
    #[error("Invalid currency settings.")]
    InvalidCurrencySettings,
    #[error("Set valid counter buy and sell rates first.")]
    RatesFirst,
    #[error("Too many requests. Please wait a moment and try again.")]
    TooManyQuotes,
}
