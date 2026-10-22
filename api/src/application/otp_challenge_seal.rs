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

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use xc_kernel::pii::PiiKeys;

type HmacSha256 = Hmac<Sha256>;
const SUBKEY_INFO: &[u8] = b"xc-otp-challenge-v1";
const VERSION: &str = "v1";

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
    fn compute_mac(
        keys: &PiiKeys,
        channel: OtpChannel,
        destination: &str,
        challenge_id: &str,
    ) -> [u8; 32] {
        let subkey = xc_kernel::pii::derive_subkey(keys, SUBKEY_INFO);
        let mut mac = HmacSha256::new_from_slice(&subkey).expect("HMAC can take key of any length");
        mac.update(VERSION.as_bytes());
        mac.update(&[0]);
        mac.update(channel.as_str().as_bytes());
        mac.update(&[0]);
        mac.update(destination.as_bytes());
        mac.update(&[0]);
        mac.update(challenge_id.as_bytes());
        mac.finalize().into_bytes().into()
    }

    /// The cookie value (wire form above). Deterministic for the same seal and keys.
    pub fn seal(&self, keys: &PiiKeys) -> String {
        let dest_b64 = URL_SAFE_NO_PAD.encode(self.destination.as_bytes());
        let mac = Self::compute_mac(keys, self.channel, &self.destination, &self.challenge_id);
        let mac_b64 = URL_SAFE_NO_PAD.encode(mac);
        format!(
            "{VERSION}.{}.{dest_b64}.{}.{mac_b64}",
            self.channel.as_str(),
            self.challenge_id
        )
    }

    /// The seal back, or `None` for anything that is not exactly a value `seal` produced with these keys: a bad
    /// shape, an unknown version or channel, bad base64, or a MAC that does not match (compared in constant time).
    pub fn open(value: &str, keys: &PiiKeys) -> Option<Self> {
        let mut parts = value.split('.');
        let version = parts.next()?;
        let channel_str = parts.next()?;
        let dest_b64 = parts.next()?;
        let challenge_id = parts.next()?;
        let mac_b64 = parts.next()?;
        if parts.next().is_some() || version != VERSION || challenge_id.is_empty() {
            return None;
        }
        let channel = OtpChannel::parse(channel_str)?;
        let dest_bytes = URL_SAFE_NO_PAD.decode(dest_b64).ok()?;
        if URL_SAFE_NO_PAD.encode(&dest_bytes) != dest_b64 {
            return None;
        }
        let destination = String::from_utf8(dest_bytes).ok()?;
        if destination.is_empty() {
            return None;
        }
        let mac_bytes = URL_SAFE_NO_PAD.decode(mac_b64).ok()?;
        if URL_SAFE_NO_PAD.encode(&mac_bytes) != mac_b64 {
            return None;
        }
        let expected = Self::compute_mac(keys, channel, &destination, challenge_id);
        if !bool::from(mac_bytes.as_slice().ct_eq(&expected)) {
            return None;
        }
        Some(Self {
            channel,
            destination,
            challenge_id: challenge_id.to_owned(),
        })
    }
}
