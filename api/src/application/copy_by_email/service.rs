//! `CopyByEmailService` (design `kiosk-shared-phone.md` §4.2 D21, D22; §4.4 B19–B25): the core of
//! the e-mail copy. Every rule is here; a `CopySubject` only loads its row.
//!
//! `[required]` order of `send`'s checks (each failure sends nothing and stores nothing):
//! 1. the subject: no `CopySubject` for `kind`, or `load_for(member, id, SHA-256(qr))` is `None`
//!    ⇒ `not_found()` (`404 notFound`);
//! 2. the member's verified e-mail (`MemberEmails`): none ⇒ `no_verified_email()`
//!    (`409 noVerifiedEmail`);
//! 3. the sender: `!is_configured()` ⇒ `email_unavailable()` (`503 emailUnavailable`); a send
//!    failure passes through as the sender reports it.
//!
//! The message: `to` = that verified address; `idempotency_key` =
//! `copy:<subject>:<id>:<lowercase hex of SHA-256(qr)>`; text and HTML in the member's `lang`
//! (th/en/zh) with the details rows, the branch, and the link `CopyLink::url(origin, sign(…, now +
//! 24 h))`; the HTML also carries the QR as `<img src="data:image/png;base64,…">` (`qr_png`).
//! Never a passport, a phone or a customer number.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_customer::domain::email_sender::{EMAIL_UNAVAILABLE, EmailSender};

use super::link::CopyLink;
use super::member_email::MemberEmails;
use super::subject::{CopyDetails, CopySubject, CopySubjectKind};

/// What `copyByEmail` answers: `{sent: true}`, or `{sent: true, again: true}` when Notify
/// recognised the idempotency key (B20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub again: bool,
}

/// What the link page shows: the details and the live status, plus the QR from the link itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyView {
    pub details: CopyDetails,
    pub qr: String,
    pub expires_at: DateTime<Utc>,
}

/// `404 notFound`: another member's, unknown, or a reissued-away QR — one answer.
pub fn not_found() -> AppError {
    AppError::NotFound {
        entity: "copySubject",
        id: String::new(),
    }
}

/// `409 noVerifiedEmail`.
pub fn no_verified_email() -> AppError {
    AppError::Conflict("noVerifiedEmail".to_owned())
}

/// `503 emailUnavailable`.
pub fn email_unavailable() -> AppError {
    AppError::Domain(EMAIL_UNAVAILABLE.to_owned())
}

/// `410 linkExpired`: every link failure, one answer.
pub fn link_expired() -> AppError {
    AppError::Validation("linkExpired".to_owned())
}

#[derive(Clone)]
pub struct CopyByEmailService {
    subjects: Vec<Arc<dyn CopySubject>>,
    members: Arc<dyn MemberEmails>,
    sender: Arc<dyn EmailSender>,
    link: CopyLink,
    origin: String,
}

impl CopyByEmailService {
    /// `origin` = the site the link opens on (`https://host`, no trailing slash needed).
    pub fn new(
        subjects: Vec<Arc<dyn CopySubject>>,
        members: Arc<dyn MemberEmails>,
        sender: Arc<dyn EmailSender>,
        link: CopyLink,
        origin: String,
    ) -> Self {
        Self {
            subjects,
            members,
            sender,
            link,
            origin,
        }
    }

    /// `copyByEmail` (B19–B23): see the module header for the order and the message.
    pub async fn send(
        &self,
        member: &str,
        kind: CopySubjectKind,
        id: &str,
        qr: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Sent> {
        let _ = (
            &self.subjects,
            &self.members,
            &self.sender,
            &self.link,
            &self.origin,
            member,
            kind,
            id,
            qr,
            now,
        );
        todo!("KWQ-D3: subject → verified e-mail → sender; one message, one key per QR")
    }

    /// `copyLinkView` (B24, B25): `CopyLink::verify`, then the subject's `load_by_link(id,
    /// SHA-256(qr))`. Any failure, a subject with no implementation included ⇒ `link_expired()`.
    pub async fn view_link(&self, link: &str, now: DateTime<Utc>) -> AppResult<CopyView> {
        let _ = (link, now);
        todo!("KWQ-D3: verify the link, load by link")
    }
}

/// The QR of `data` as a PNG (the e-mail's inline picture): `qrcode` for the modules, `png` for the
/// file, black on white with a 4-module quiet zone. The same data always gives the same bytes.
pub fn qr_png(data: &str) -> Vec<u8> {
    let _ = data;
    todo!("KWQ-D3: qrcode + png")
}
