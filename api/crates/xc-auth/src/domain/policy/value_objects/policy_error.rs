//! Policy refusal errors.

use thiserror::Error;

/// Errors returned when a policy check refuses an action.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyError {
    /// R1: กรุณาเข้าสู่ระบบ
    #[error("กรุณาเข้าสู่ระบบ")]
    SignIn,

    /// R2: เฉพาะเจ้าของระบบเท่านั้น
    #[error("เฉพาะเจ้าของระบบเท่านั้น")]
    OwnerOnly,

    /// R3: EMPLOYEE_AUTH_REQUIRED
    #[error("EMPLOYEE_AUTH_REQUIRED")]
    EmployeeAuthRequired,

    /// R4: คำขอไม่ถูกต้อง
    #[error("คำขอไม่ถูกต้อง")]
    InvalidRequest,

    /// R5: บทบาทนี้ไม่มีสิทธิ์ทำรายการ
    #[error("บทบาทนี้ไม่มีสิทธิ์ทำรายการ")]
    RoleNotAllowed,

    /// R6: บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้
    #[error("บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้")]
    RoleNotAllowedHere,

    /// R7: เจ้าของเป็นผู้เปิดใช้ขั้นตอน
    #[error("เจ้าของเป็นผู้เปิดใช้ขั้นตอน")]
    OwnerEnables,

    /// R8: ใช้ได้เฉพาะ Front
    #[error("ใช้ได้เฉพาะ Front")]
    FrontOnly,

    /// R9: บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้า
    #[error("บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้า")]
    DocumentsNotReadable,

    /// R10: บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้าหรือหลักฐานเงิน
    #[error("บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้าหรือหลักฐานเงิน")]
    FilesNotReadable,

    /// R11: บทบาทนี้อัปโหลดหลักฐานไม่ได้
    #[error("บทบาทนี้อัปโหลดหลักฐานไม่ได้")]
    UploadNotAllowed,

    /// R12: Not found
    #[error("Not found")]
    NotFound,

    /// R13: บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้
    #[error("บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้")]
    DeviceStepNotAllowed,

    /// R14: Owner or Rates access required.
    #[error("Owner or Rates access required.")]
    KioskOwnerOrRates,

    /// R15: Staff access required.
    #[error("Staff access required.")]
    KioskStaff,

    /// R16: Unknown kiosk action.
    #[error("Unknown kiosk action.")]
    KioskUnknown,

    /// R17: ไม่มีสิทธิ์ลงชื่อ
    #[error("ไม่มีสิทธิ์ลงชื่อ")]
    SignNotAllowed,

    /// R18: Backend ใช้ปุ่มตรวจและบันทึกงาน
    #[error("Backend ใช้ปุ่มตรวจและบันทึกงาน")]
    BackendUsesCommit,

    /// R19: ต้องใช้บัญชีพนักงาน
    #[error("ต้องใช้บัญชีพนักงาน")]
    EmployeeAccount,

    /// R20: ลงเวลาเข้าก่อนทำรายการ
    #[error("ลงเวลาเข้าก่อนทำรายการ")]
    ClockInFirst,
}

impl PolicyError {
    /// Returns the HTTP status code associated with this refusal.
    pub fn status(&self) -> u16 {
        match self {
            Self::SignIn | Self::EmployeeAuthRequired => 401,
            Self::OwnerOnly => 403,
            Self::NotFound => 404,
            Self::InvalidRequest
            | Self::RoleNotAllowed
            | Self::RoleNotAllowedHere
            | Self::OwnerEnables
            | Self::FrontOnly
            | Self::DocumentsNotReadable
            | Self::FilesNotReadable
            | Self::UploadNotAllowed
            | Self::DeviceStepNotAllowed
            | Self::KioskOwnerOrRates
            | Self::KioskStaff
            | Self::KioskUnknown
            | Self::SignNotAllowed
            | Self::BackendUsesCommit
            | Self::EmployeeAccount
            | Self::ClockInFirst => 400,
        }
    }
}
