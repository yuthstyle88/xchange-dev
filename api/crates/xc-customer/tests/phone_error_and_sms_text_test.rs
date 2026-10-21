use pos_error::AppError;
use xc_customer::{PhoneOtpPolicy, PhoneVerificationError};

#[test]
fn test_row_1_internal_code_and_status() {
    let err = PhoneVerificationError::Internal("x".to_string());
    assert_eq!(err.code(), "internal");
    assert_eq!(err.status_code(), 500);
}

#[test]
fn test_row_2_database_code_and_status() {
    let err = PhoneVerificationError::Database;
    assert_eq!(err.code(), "database");
    assert_eq!(err.status_code(), 500);
}

#[test]
fn test_row_3_from_internal_error_to_app_error() {
    let app_err = AppError::from(PhoneVerificationError::Internal("x".to_string()));
    match app_err {
        AppError::Internal(msg) => assert_eq!(msg, "x"),
        other => panic!("expected AppError::Internal, got {other:?}"),
    }
}

#[test]
fn test_row_4_from_wrong_code_error_to_app_error() {
    let app_err = AppError::from(PhoneVerificationError::WrongCode);
    match app_err {
        AppError::Domain(code) => assert_eq!(code, "wrongCode"),
        other => panic!("expected AppError::Domain, got {other:?}"),
    }
}

#[test]
fn test_row_5_from_app_error_to_phone_verification_error() {
    let err = PhoneVerificationError::from(AppError::Domain("test".to_string()));
    assert_eq!(
        err,
        PhoneVerificationError::Internal("domain rule violation: test".to_string())
    );
}

#[test]
fn test_row_6_sms_text_fallback_to_thai() {
    assert_eq!(
        PhoneOtpPolicy::sms_text("fr", "123456", "Siam"),
        "รหัสยืนยันของคุณคือ 123456 ที่ Siam"
    );
}

#[test]
fn test_row_7_sms_text_en_template() {
    assert_eq!(
        PhoneOtpPolicy::sms_text("en", "123456", "Siam"),
        "Your verification code is 123456 for Siam"
    );
}

#[test]
fn test_row_8_sms_text_case_sensitive_matching() {
    assert_eq!(
        PhoneOtpPolicy::sms_text("EN", "123456", "Siam"),
        "รหัสยืนยันของคุณคือ 123456 ที่ Siam"
    );
}
