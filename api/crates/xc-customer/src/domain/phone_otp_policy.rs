//! Phone OTP policy defaults and SMS templates (CU-P1, design §6.3).

pub struct PhoneOtpPolicy;

impl PhoneOtpPolicy {
    pub const CODE_DIGITS: usize = 6;
    pub const EXPIRY_MINUTES: i64 = 5;
    pub const MAX_ATTEMPTS: i32 = 5;
    pub const MAX_PER_CUSTOMER_WINDOW_MINUTES: i64 = 15;
    pub const MAX_PER_CUSTOMER_CHALLENGES: i64 = 3;
    pub const MAX_PER_PHONE_WINDOW_HOURS: i64 = 1;
    pub const MAX_PER_PHONE_CHALLENGES: i64 = 5;
    pub const MAX_PER_TELLER_WINDOW_HOURS: i64 = 1;
    pub const MAX_PER_TELLER_CHALLENGES: i64 = 30;

    /// Formats the SMS text in the customer's preferred language (th, en, zh).
    /// Holds only the code and the shop name.
    pub fn sms_text(lang: &str, code: &str, shop_name: &str) -> String {
        match lang {
            "en" => format!("Your verification code is {code} for {shop_name}"),
            "zh" => format!("您的验证码是 {code}（{shop_name}）"),
            _ => format!("รหัสยืนยันของคุณคือ {code} ที่ {shop_name}"),
        }
    }
}
