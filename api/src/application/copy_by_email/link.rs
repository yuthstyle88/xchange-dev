//! `CopyLink` (design `kiosk-shared-phone.md` §4.2 D22): the signed, unstored 1-day link
//! `<origin>/c#<link>`, `<link>` = `v1.<subject>.<id>.<exp>.<qr>.<mac>`, `exp` = sent + 24 h (unix
//! seconds), `mac` = HMAC-SHA256 under `derive_subkey(pii_keys, "xc-copy-link-v1")` over the
//! subject, id, exp and qr (`[proposed]`: each part separated so no two links share a MAC input;
//! base64url, no padding). No table and no env secret.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use subtle::ConstantTimeEq as _;
use xc_kernel::pii::{PiiKeys, hmac_sha256};

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

    fn mac_b64(&self, kind: CopySubjectKind, id: &str, exp_str: &str, qr: &str) -> String {
        let mac = hmac_sha256(
            &self.key,
            &[
                b"v1",
                b".",
                kind.as_str().as_bytes(),
                b".",
                id.as_bytes(),
                b".",
                exp_str.as_bytes(),
                b".",
                qr.as_bytes(),
            ],
        );
        URL_SAFE_NO_PAD.encode(mac)
    }

    /// The link for `kind`/`id`/`qr`, valid until `expires_at` (whole seconds).
    pub fn sign(
        &self,
        kind: CopySubjectKind,
        id: &str,
        qr: &str,
        expires_at: DateTime<Utc>,
    ) -> String {
        let exp_str = expires_at.timestamp().to_string();
        let mac = self.mac_b64(kind, id, &exp_str, qr);
        format!("v1.{}.{id}.{exp_str}.{qr}.{mac}", kind.as_str())
    }

    /// Checks, in order: the shape, the MAC (constant time), then `expires_at > now`.
    pub fn verify(&self, link: &str, now: DateTime<Utc>) -> Result<LinkParts, LinkRejected> {
        let [version, subject_str, id, exp_str, qr, mac_str]: [&str; 6] = link
            .split('.')
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| LinkRejected)?;
        if version != "v1" || id.is_empty() || qr.is_empty() {
            return Err(LinkRejected);
        }
        let kind = CopySubjectKind::parse(subject_str).ok_or(LinkRejected)?;
        let exp_secs: i64 = exp_str.parse().map_err(|_| LinkRejected)?;
        let expires_at = DateTime::from_timestamp(exp_secs, 0).ok_or(LinkRejected)?;

        let expected_mac = self.mac_b64(kind, id, exp_str, qr);
        if !bool::from(mac_str.as_bytes().ct_eq(expected_mac.as_bytes())) {
            return Err(LinkRejected);
        }

        if expires_at <= now {
            return Err(LinkRejected);
        }

        Ok(LinkParts {
            kind,
            id: id.to_owned(),
            qr: qr.to_owned(),
            expires_at,
        })
    }

    /// `<origin>/c#<link>`.
    pub fn url(origin: &str, link: &str) -> String {
        format!("{}/c#{link}", origin.trim_end_matches('/'))
    }
}
