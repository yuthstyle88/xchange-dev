//! Shift error domain value object.

use xc_kernel::money::CurrencyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShiftError {
    #[error("ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด")]
    TextInvalid,
    #[error("รหัสคำขอไม่ถูกต้อง")]
    RequestIdInvalid,
    #[error("กรุณากรอกจำนวนเงิน")]
    AmountMissing,
    #[error("จำนวนเงินต้องไม่ติดลบ ไม่เกิน 100,000,000 และมีทศนิยมไม่เกิน 2 ตำแหน่ง")]
    AmountInvalid,
    #[error("รายการรับ\u{2013}จ่ายไม่ถูกต้อง")]
    MovementInvalid,
    #[error("ไม่พบสาขาที่เลือก กรุณารีเฟรช")]
    BranchNotFound,
    #[error("คำขอนี้ถูกยกเลิกแล้ว กรุณาโหลดข้อมูลล่าสุด")]
    RequestAbandoned,
    #[error("คำขอเดิมมีข้อมูลเปลี่ยน")]
    RequestChanged,
    #[error("มีกะเปิดอยู่แล้ว กรุณารีเฟรช")]
    ShiftAlreadyOpen,
    #[error("กะเปลี่ยนหรือปิดแล้ว กรุณารีเฟรช")]
    ShiftChanged,
    #[error("กรุณาเปิดกะจากหน้าแอดมินก่อน")]
    NoOpenShift,
    #[error("ยอด {0} ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน")]
    Insufficient(CurrencyCode),
    #[error("ยอดเปลี่ยนระหว่างตรวจนับ กรุณารีเฟรชและตรวจสอบอีกครั้ง")]
    ExpectedChanged,
    #[error("กรุณาระบุเหตุผลยอดต่างก่อนปิดกะ")]
    DifferenceNeedsNote,
}

impl From<ShiftError> for pos_error::AppError {
    fn from(e: ShiftError) -> Self {
        pos_error::AppError::Domain(e.to_string())
    }
}
