//! `CopyLink` (design `kiosk-shared-phone.md` §4.2 D22): the signed, unstored 1-day link
//! `<origin>/c#<link>`, `<link>` = `v1.<subject>.<id>.<exp>.<qr>.<mac>`, `exp` = sent + 24 h (unix
//! seconds), `mac` = HMAC-SHA256 under `derive_subkey(pii_keys, "xc-copy-link-v1")` over the
//! subject, id, exp and qr (`[proposed]`: each part separated so no two links share a MAC input;
//! base64url, no padding). No table and no env secret.

use chrono::{DateTime, Utc};
use xc_kernel::pii::PiiKeys;

use super::subject::CopySubjectKind;

/// How long a link lives.
pub const LINK_LIFETIME_HOURS: i64 = 24;

/// What a verified link names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkParts {
    pub kind: CopySubjectKind,
    pub id: String,
    /// The QR in the clear, as the subject's page holds it.
    pub qr: String,
    pub expires_at: DateTime<Utc>,
}

/// Any failure of `verify`: one answer for all (`410 linkExpired`), so a forged, expired or
/// malformed link cannot be told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkRejected;

#[derive(Clone)]
pub struct CopyLink {
    key: [u8; 32],
}

impl CopyLink {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// The key derived from the PII keys (info `xc-copy-link-v1`), the booking QR's pattern.
    pub fn from_pii(keys: &PiiKeys) -> Self {
        Self::new(xc_kernel::pii::derive_subkey(keys, b"xc-copy-link-v1"))
    }

    /// The link for `kind`/`id`/`qr`, valid until `expires_at` (whole seconds).
    pub fn sign(
        &self,
        kind: CopySubjectKind,
        id: &str,
        qr: &str,
        expires_at: DateTime<Utc>,
    ) -> String {
        let _ = (&self.key, kind, id, qr, expires_at);
        todo!("KWQ-D2: v1.<subject>.<id>.<exp>.<qr>.<mac>")
    }

    /// Checks, in order: the shape, the MAC (constant time), then `expires_at > now`.
    pub fn verify(&self, link: &str, now: DateTime<Utc>) -> Result<LinkParts, LinkRejected> {
        let _ = (&self.key, link, now);
        todo!("KWQ-D2: parse, constant-time MAC, expiry")
    }

    /// `<origin>/c#<link>`.
    pub fn url(origin: &str, link: &str) -> String {
        format!("{}/c#{link}", origin.trim_end_matches('/'))
    }
}
