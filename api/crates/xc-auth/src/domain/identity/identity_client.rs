use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

pub const UNAVAILABLE_MESSAGE: &str = "ระบบยืนยันตัวตนไม่พร้อมใช้งาน กรุณาลองใหม่ภายหลัง";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityLoginResponse {
    #[serde(rename = "identityId")]
    pub identity_id: String,
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default, rename = "emailVerified")]
    pub email_verified: Option<bool>,
    #[serde(default, rename = "phoneVerified")]
    pub phone_verified: Option<bool>,
    #[serde(default, rename = "mustChangePassword")]
    pub must_change_password: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityOtpRequestResponse {
    #[serde(rename = "challengeId")]
    pub challenge_id: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityOtpVerifyResponse {
    pub verified: bool,
    #[serde(default)]
    pub login: Option<IdentityLoginResponse>,
    #[serde(default)]
    pub registered: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityRegisterResponse {
    #[serde(default, rename = "identityId")]
    pub identity_id: Option<String>,
    #[serde(default)]
    pub registered: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    InvalidCredentials,
    AccountLocked,
    TooManyAttempts,
    IdentityDisabled,
    InvalidCode,
    CodeExpired,
    SignIn,
    AlreadyRegistered,
    PhoneTaken,
    UsernameTaken,
    EmailTaken,
    PhoneRequired,
    InvalidPhone,
    WeakPassword,
    InvalidRequest,
    Unavailable {
        path: String,
        status: u16,
        code: String,
    },
}

impl IdentityError {
    pub fn unavailable(path: impl Into<String>, status: u16, code: impl Into<String>) -> Self {
        Self::Unavailable {
            path: path.into(),
            status,
            code: code.into(),
        }
    }

    pub fn refusal(&self) -> (u16, &'static str, &'static str) {
        match self {
            Self::InvalidCredentials => (401, "invalidCredentials", "ชื่อผู้ใช้หรือรหัสผ่านไม่ถูกต้อง"),
            Self::AccountLocked => (423, "accountLocked", "บัญชีถูกล็อกชั่วคราว กรุณาลองใหม่ภายหลัง"),
            Self::TooManyAttempts => (
                429,
                "tooManyAttempts",
                "ลองหลายครั้งเกินไป กรุณารอสักครู่แล้วลองใหม่",
            ),
            Self::IdentityDisabled => (403, "identityDisabled", "บัญชีนี้ถูกระงับที่ระบบยืนยันตัวตน"),
            Self::InvalidCode => (401, "invalidCode", "รหัสยืนยันไม่ถูกต้อง"),
            Self::CodeExpired => (410, "codeExpired", "รหัสยืนยันหมดอายุ กรุณาขอรหัสใหม่"),
            Self::SignIn => (401, "signIn", "กรุณาเข้าสู่ระบบ"),
            Self::AlreadyRegistered => (409, "alreadyRegistered", "มีบัญชีนี้อยู่แล้ว กรุณาเข้าสู่ระบบ"),
            Self::PhoneTaken => (400, "phoneTaken", "เบอร์โทรศัพท์นี้มีบัญชีอยู่แล้ว"),
            Self::UsernameTaken => (400, "usernameTaken", "ชื่อผู้ใช้นี้มีอยู่แล้ว"),
            Self::EmailTaken => (400, "emailTaken", "อีเมลนี้มีบัญชีอยู่แล้ว"),
            Self::PhoneRequired => (422, "phoneRequired", "กรุณากรอกเบอร์โทรศัพท์"),
            Self::InvalidPhone => (422, "invalidPhone", "เบอร์โทรศัพท์ไม่ถูกต้อง"),
            Self::WeakPassword => (422, "weakPassword", "รหัสผ่านไม่ผ่านเงื่อนไขความปลอดภัย"),
            Self::InvalidRequest => (400, "invalidRequest", "คำขอไม่ถูกต้อง"),
            Self::Unavailable { .. } => (503, "identityUnavailable", UNAVAILABLE_MESSAGE),
        }
    }
}

pub trait IdentityClient: Send + Sync + 'static {
    fn login<'a>(
        &'a self,
        identifier: &'a str,
        password: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityLoginResponse, IdentityError>>;

    fn refresh<'a>(
        &'a self,
        refresh_token: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityLoginResponse, IdentityError>>;

    fn logout<'a>(
        &'a self,
        refresh_token: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), IdentityError>>;

    fn otp_request<'a>(
        &'a self,
        destination: &'a str,
        channel: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityOtpRequestResponse, IdentityError>>;

    fn otp_verify<'a>(
        &'a self,
        challenge_id: &'a str,
        code: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityOtpVerifyResponse, IdentityError>>;

    /// Creates an Identity account server-side (#1157). Only the staff create and the
    /// `create-owner` bootstrap call this; no public route reaches it. No OTP is sent.
    fn register<'a>(
        &'a self,
        request: &'a IdentityRegisterRequest<'a>,
    ) -> BoxFuture<'a, Result<IdentityRegisterResponse, IdentityError>>;

    fn change_password<'a>(
        &'a self,
        access_token: &'a str,
        current_password: &'a str,
        new_password: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), IdentityError>>;
}

/// What a server-side create sends to Identity `POST /auth/register` (camelCase on the wire).
#[derive(Debug, Clone, Copy)]
pub struct IdentityRegisterRequest<'a> {
    pub username: Option<&'a str>,
    pub phone: &'a str,
    pub email: Option<&'a str>,
    pub password: &'a str,
    pub must_change_password: bool,
}
