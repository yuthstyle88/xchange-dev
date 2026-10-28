use pos_error::AppError;
use xc_auth::domain::access::Refusal;
use xc_cash::domain::count::RejectError;
use xc_cash::domain::error::CashError;

#[test]
fn reject_error_message_test() {
    let other_err = AppError::Domain("ฐานข้อมูลขัดข้อง".to_owned());
    let other_want = other_err.to_string();
    let cases = [
        (
            RejectError::Refused(Refusal::SignIn),
            "ไม่มีสิทธิ์ปฏิเสธการนับของสาขานี้".to_owned(),
        ),
        (
            RejectError::Refused(Refusal::OutOfScope),
            "ไม่มีสิทธิ์ปฏิเสธการนับของสาขานี้".to_owned(),
        ),
        (
            RejectError::Refused(Refusal::RoleNotAllowed),
            "ไม่มีสิทธิ์ปฏิเสธการนับของสาขานี้".to_owned(),
        ),
        (RejectError::NotFound, "ไม่พบการนับนี้".to_owned()),
        (
            RejectError::OnlyThroughHandover,
            "การนับส่งมอบต้องจัดการผ่านการส่งมอบเท่านั้น".to_owned(),
        ),
        (
            RejectError::SameCounter,
            CashError::SameCounter.to_string(),
        ),
        (RejectError::NotWaiting, CashError::NotWaiting.to_string()),
        (RejectError::ReasonRequired, "กรุณาระบุเหตุผล".to_owned()),
        (
            RejectError::ReasonTooLong,
            "เหตุผลยาวเกิน 500 ตัวอักษร".to_owned(),
        ),
        (RejectError::Other(other_err), other_want),
    ];

    for (err, want) in cases {
        let msg = err.message();
        assert_eq!(msg, want, "{err:?}");
        assert!(!msg.is_empty(), "{err:?}");
        assert_eq!(msg.trim(), msg, "{err:?}");
        assert_ne!(msg, err.code(), "{err:?}");
    }
}
