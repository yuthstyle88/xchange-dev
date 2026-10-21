#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KioskError {
    #[error("Counter is closed. Please contact our staff.")]
    CounterClosed,
    #[error("Select an exchange direction.")]
    DirectionRequired,
    #[error("Please enter a valid amount and currency.")]
    AmountOrCurrencyInvalid,
    #[error("Amount is too small.")]
    AmountTooSmall,
    #[error("Select cash or bank transfer to receive THB.")]
    BuyPaidInCashOrTransfer,
    #[error("Invalid payment method.")]
    PaymentMethodInvalid,
    #[error("Request not found. Please start again.")]
    RequestNotFoundStartAgain,
    #[error("Quote expired. Please check the rate again.")]
    QuoteExpired,
    #[error("Please review your quote for 3 seconds.")]
    ReviewThreeSeconds,
    #[error("Rate changed or counter closed. Please start again.")]
    RateChangedOrClosed,
    #[error("Request not found.")]
    RequestNotFound,
    #[error("Staff access required.")]
    StaffRequired,
    #[error("The assigned employee must confirm the passport check.")]
    PassportCheckUnconfirmed,
    #[error("Assigned staff required.")]
    AssignedStaffRequired,
    #[error("Accept this request using your own account first.")]
    AcceptFirst,
    #[error("This request is already assigned or unavailable.")]
    AlreadyAssignedOrUnavailable,
    #[error("คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว")]
    NotReadyToSave,
    #[error("คิวต้องบันทึกเป็นรายการจริง")]
    SaveMustBeLive,
    #[error("สกุลเงินและจำนวนต้องตรงกับคิว")]
    SaveDiffersFromQueue,
    #[error("ช่องทางต้องตรงกับที่ลูกค้าเลือก")]
    SaveMethodDiffers,
}
