use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;
use xc_kernel::pii::{PiiCipher, PiiKeys, Sealed};

/// S-3: a link lives 60 seconds (owner, 2026-10-11).
pub const LINK_LIFETIME: Duration = Duration::seconds(60);

const AAD: &[u8] = b"xc-verification-download-link-v1";

/// A signed link: the token to put in the URL and when it stops working.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedLink {
    /// URL-safe text: no `/`, `?`, `#` or whitespace.
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// What a valid token names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkClaims {
    pub document_id: Uuid,
    /// The `users.id` the link was issued to.
    pub actor_id: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// Used after its lifetime (`linkExpired`).
    #[error("the link has expired")]
    Expired,
    /// Used by a session other than the one it was issued to.
    #[error("the link belongs to another session")]
    OtherSession,
    /// Not a token this server signed: tampered, truncated or signed with another key.
    #[error("the link is not valid")]
    Invalid,
}

pub struct DownloadLink {
    keys: Arc<PiiKeys>,
}

impl DownloadLink {
    pub fn new(keys: Arc<PiiKeys>) -> Self {
        Self { keys }
    }

    /// A token for `document_id`, issued to `actor_id` in `session` at `now`; it expires at `now + LINK_LIFETIME`.
    pub fn sign(
        &self,
        document_id: Uuid,
        actor_id: &str,
        session: &str,
        now: DateTime<Utc>,
    ) -> SignedLink {
        let expires_at = now + LINK_LIFETIME;
        let payload = serde_json::json!({
            "document_id": document_id.to_string(),
            "actor_id": actor_id,
            "session": session,
            "expires_at": expires_at.to_rfc3339(),
        });
        let json_bytes = serde_json::to_vec(&payload).expect("serialize payload");
        let cipher = PiiCipher::new(self.keys.clone());
        let sealed = cipher.encrypt(AAD, &json_bytes);
        let hex_bytes = hex_encode(&sealed.bytes);
        let token = format!("{}.{}", sealed.key_id, hex_bytes);
        SignedLink { token, expires_at }
    }

    /// The claims of `token` when this server signed it for `session` and it has not expired at `now`.
    pub fn verify(
        &self,
        token: &str,
        session: &str,
        now: DateTime<Utc>,
    ) -> Result<LinkClaims, LinkError> {
        let (key_id, hex_bytes) = token.split_once('.').ok_or(LinkError::Invalid)?;
        if key_id.is_empty() || hex_bytes.is_empty() {
            return Err(LinkError::Invalid);
        }
        let raw_bytes = hex_decode(hex_bytes).ok_or(LinkError::Invalid)?;
        let sealed = Sealed {
            key_id: key_id.to_string(),
            bytes: raw_bytes,
        };
        let cipher = PiiCipher::new(self.keys.clone());
        let decrypted = cipher
            .decrypt(AAD, &sealed)
            .map_err(|_| LinkError::Invalid)?;
        let val: serde_json::Value =
            serde_json::from_slice(&decrypted).map_err(|_| LinkError::Invalid)?;

        let doc_id_str = val
            .get("document_id")
            .and_then(|v| v.as_str())
            .ok_or(LinkError::Invalid)?;
        let document_id = Uuid::parse_str(doc_id_str).map_err(|_| LinkError::Invalid)?;

        let actor_id = val
            .get("actor_id")
            .and_then(|v| v.as_str())
            .ok_or(LinkError::Invalid)?
            .to_string();

        let token_session = val
            .get("session")
            .and_then(|v| v.as_str())
            .ok_or(LinkError::Invalid)?;

        let exp_str = val
            .get("expires_at")
            .and_then(|v| v.as_str())
            .ok_or(LinkError::Invalid)?;
        let expires_at = DateTime::parse_from_rfc3339(exp_str)
            .map_err(|_| LinkError::Invalid)?
            .with_timezone(&Utc);

        if token_session != session {
            return Err(LinkError::OtherSession);
        }
        if now > expires_at {
            return Err(LinkError::Expired);
        }

        Ok(LinkClaims {
            document_id,
            actor_id,
            expires_at,
        })
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02X}");
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks_exact(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        bytes.push((hi << 4) | lo);
    }
    Some(bytes)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
