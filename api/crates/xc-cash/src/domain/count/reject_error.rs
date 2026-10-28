//! Staff-corrections §4 B, §5: why `countReject` refuses, each with its wire code.

use pos_error::AppError;
use xc_auth::domain::access::Refusal;

/// The answer of [`crate::application::count::count_service::CashCountService::reject_in`] when it refuses.
#[derive(Debug)]
pub enum RejectError {
    /// B1: no `ShiftCloseCertify` on the count's branch (`roleNotAllowed`, `outOfScope`, `signIn`).
    Refused(Refusal),
    /// B2: no count with that id.
    NotFound,
    /// B3: a `HANDOVER_OUT` count; handovers settle their own counts.
    OnlyThroughHandover,
    /// B4: the caller counted it.
    SameCounter,
    /// B5: already verified or already rejected.
    NotWaiting,
    /// B6: the reason is blank after trim.
    ReasonRequired,
    /// B6: the reason is longer than 500 UTF-16 units.
    ReasonTooLong,
    /// Anything else (database, …), unchanged.
    Other(AppError),
}

impl RejectError {
    /// The v3 refusal code (§5); `Other` has none of its own.
    pub fn code(&self) -> &'static str {
        match self {
            RejectError::Refused(r) => r.code(),
            RejectError::NotFound => "notFound",
            RejectError::OnlyThroughHandover => "onlyThroughHandover",
            RejectError::SameCounter => "sameCounter",
            RejectError::NotWaiting => "notWaiting",
            RejectError::ReasonRequired => "reasonRequired",
            RejectError::ReasonTooLong => "validationFailed",
            RejectError::Other(_) => "internal",
        }
    }

    /// The sentence the client shows for this refusal (§5). Until the follow-up ticket writes the sentences, the code.
    pub fn message(&self) -> String {
        match self {
            RejectError::Refused(_) => "ไม่มีสิทธิ์ปฏิเสธการนับของสาขานี้".to_owned(),
            RejectError::NotFound => "ไม่พบการนับนี้".to_owned(),
            RejectError::OnlyThroughHandover => "การนับส่งมอบต้องจัดการผ่านการส่งมอบเท่านั้น".to_owned(),
            RejectError::SameCounter => crate::domain::error::CashError::SameCounter.to_string(),
            RejectError::NotWaiting => crate::domain::error::CashError::NotWaiting.to_string(),
            RejectError::ReasonRequired => "กรุณาระบุเหตุผล".to_owned(),
            RejectError::ReasonTooLong => "เหตุผลยาวเกิน 500 ตัวอักษร".to_owned(),
            RejectError::Other(e) => e.to_string(),
        }
    }
}

impl From<AppError> for RejectError {
    fn from(e: AppError) -> Self {
        RejectError::Other(e)
    }
}
