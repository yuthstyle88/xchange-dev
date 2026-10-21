//! `OtpChallengeSeal` (design `kiosk-shared-phone.md` §4.2 D11; the KWQ-C2 skeleton's [proposed] seam): which
//! destination an OTP challenge was sent to, carried from `POST /api/auth/otp/request` to `…/otp/verify` without a
//! table.
//!
//! Why it exists: Identity's login answer names the phone (`phone`, `phoneVerified`) but leaves `email` unset for an
//! OTP login (`108-platform-services` `681bea2` `crates/auth/src/contract/mod.rs` `LoginResponse.email`: "Password/OTP
//! logins leave this unset (the caller already has the identifier it submitted)"). So the BFF must remember the
//! e-mail it sent the code to, and it must not take it from the verify request, where a caller could name someone
//! else's address. The request route seals `{channel, destination, challengeId}` with
//! `HMAC-SHA256(derive_subkey(pii_keys, "xc-otp-challenge-v1"), …)` into the HttpOnly cookie [`OTP_COOKIE`]; the
//! verify route opens it and trusts the destination only when the MAC holds and the challenge id is the one being
//! verified. Nothing is stored; no new secret.
//!
//! Wire form `[required]`: `v1.<channel>.<b64url(destination)>.<challengeId>.<b64url(mac)>`, the MAC over
//! `"v1" ‖ 0 ‖ channel ‖ 0 ‖ destination ‖ 0 ‖ challengeId`. `channel` is `sms` or `email`. `destination` is the
//! normalised value (`xc_kernel::pii::normalize`: E.164 for a phone, the normalised e-mail).

use xc_kernel::pii::PiiKeys;

/// The cookie the seal travels in: `__Host-xc_otp`, `HttpOnly; Secure; SameSite=Strict; Path=/`, `Max-Age` = the
/// challenge's lifetime (at most 600 s); cleared by the verify route on any answer.
pub const OTP_COOKIE: &str = "__Host-xc_otp";

/// Which channel the code went by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtpChannel {
    Sms,
    Email,
}

impl OtpChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sms => "sms",
            Self::Email => "email",
        }
    }

    /// `sms` / `email`; anything else `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "sms" => Some(Self::Sms),
            "email" => Some(Self::Email),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtpChallengeSeal {
    pub channel: OtpChannel,
    /// Normalised: E.164 for `Sms`, the normalised address for `Email`.
    pub destination: String,
    pub challenge_id: String,
}

impl OtpChallengeSeal {
    /// The cookie value (wire form above). Deterministic for the same seal and keys.
    pub fn seal(&self, keys: &PiiKeys) -> String {
        let _ = keys;
        todo!("KWQ-C2-3: sign the seal")
    }

    /// The seal back, or `None` for anything that is not exactly a value `seal` produced with these keys: a bad
    /// shape, an unknown version or channel, bad base64, or a MAC that does not match (compared in constant time).
    pub fn open(value: &str, keys: &PiiKeys) -> Option<Self> {
        let _ = (value, keys);
        todo!("KWQ-C2-3: verify the seal")
    }
}
