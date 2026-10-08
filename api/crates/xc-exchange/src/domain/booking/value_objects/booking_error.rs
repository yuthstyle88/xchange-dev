//! Booking error types and refusal sentences.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BookingError {
    #[error("กรอกข้อมูลให้ครบและไม่เกินกำหนด")]
    FillIn,
    #[error("เบอร์ติดต่อไม่ถูกต้อง")]
    BadPhone,
    #[error("รหัสสมาชิกไม่ถูกต้อง")]
    BadMemberCode,
    #[error("กำหนดเวลา 5\u{2013}1440 นาที")]
    BadRules,
    #[error("ไม่พบ Member ของลูกค้านี้ในสาขานี้")]
    NoMember,
    #[error("ปิดรับจองใหม่")]
    Closed,
    #[error("กรุณาเปิดกะจากหน้าแอดมินก่อน")]
    NoShift,
    #[error("เรทเปลี่ยน กรุณาโหลดใหม่")]
    RateMoved,
    #[error("วันหมดอายุต้องอยู่ในอนาคต")]
    ExpiryInFuture,
    #[error("ยืนยันได้เฉพาะคำขอ Member ที่รอยืนยัน")]
    OnlyPendingMember,
    #[error("การจองถูกใช้ทำรายการแล้ว")]
    AlreadyUsed,
    #[error("การจองนี้ถูกใช้ทำรายการแล้ว")]
    UsedByAnother,
    #[error("การจองยังไม่ยืนยัน หมดอายุ หรือถูกยกเลิกแล้ว")]
    NotUsable,
    #[error("ข้อมูลรายการไม่ตรงกับการจอง หรือยังไม่ยืนยันลูกค้า")]
    FormMismatch,
    #[error("ลูกค้าไม่ตรงกับ Member ที่จอง")]
    MemberMismatch,
    #[error("ไม่พบการจอง")]
    NotFound,
    #[error("ข้อมูลรายการไม่ครบ")]
    IncompleteForm,
    #[error("จำนวนเงินหรือเรทไม่ถูกต้อง")]
    InvalidAmountOrRate,
    #[error("ทศนิยมเกินกำหนด")]
    TooManyDecimals,
    #[error("ยอดเงินสูงเกินขอบเขตระบบ")]
    AmountOutOfRange,
    #[error("รายการเดิมมีข้อมูลเปลี่ยน ห้ามใช้รหัสส่งซ้ำกับข้อมูลใหม่")]
    Changed,
    #[error("สถานะเปลี่ยน กรุณาโหลดใหม่")]
    StatusMoved,
    #[error("ต้องเลือกลูกค้าและยืนยันตรวจตัวตนหน้าร้านก่อน")]
    CustomerNotIdentified,
    #[error("ประวัติจองเกิน 1,000 รายการ กรุณาติดต่อผู้ดูแลเพื่อเพิ่มการแบ่งหน้า")]
    HistoryTooLong,
}

impl BookingError {
    pub const fn status(&self) -> u16 {
        400
    }
}
