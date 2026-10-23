//! Shared upload check for verification documents (`CaseService::add_document` and `BranchIntake::open`).

use crate::domain::case::case_error::CaseError;

const MAX_UPLOAD_BYTES: usize = 15 * 1024 * 1024;
const EMPTY_FILE: &str = "the file is empty";
const TOO_LARGE: &str = "ไฟล์ใหญ่เกิน 15 MB";
const UNSUPPORTED: &str = "ชนิดไฟล์ไม่รองรับ";

pub(crate) type Mime = &'static str;

pub(crate) fn check_upload(bytes: &[u8]) -> Result<Mime, CaseError> {
    if bytes.is_empty() {
        return Err(CaseError::File(EMPTY_FILE));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(CaseError::File(TOO_LARGE));
    }

    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Ok("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        Ok("image/jpeg")
    } else if bytes.starts_with(b"%PDF-") {
        Ok("application/pdf")
    } else {
        Err(CaseError::File(UNSUPPORTED))
    }
}
