//! `EmailSender` (design `kiosk-shared-phone.md` §4.2 D21): the port the e-mail copy of a booking
//! or a web request is sent through. `NotifyEmailSender` (root package) implements it with
//! Notify's `POST /api/v1/notifications`, `channel: "email"`; the tests use a fake.

use pos_error::AppResult;

use crate::domain::sms_sender::BoxFuture;

/// One e-mail. `to` is always the member's verified, ACTIVE `EMAIL` identity, never a typed
/// address (the core decides it; nothing here checks it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    /// The plain-text body.
    pub text: String,
    /// The HTML body; it carries the QR as an inline `data:image/png;base64,` picture.
    pub html: String,
    /// Notify's `idempotency_key`: the same key sends one e-mail.
    pub idempotency_key: String,
}

/// What Notify answered for a sent message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailOutcome {
    /// `201`: a new notification was created.
    Sent,
    /// `200`: the idempotency key matched an earlier one; nothing new is sent.
    AlreadySent,
}

/// `Domain("emailUnavailable")`: the sender is not configured, or Notify failed (network or 5xx).
/// Answered as `503 emailUnavailable`.
pub const EMAIL_UNAVAILABLE: &str = "emailUnavailable";

pub trait EmailSender: Send + Sync {
    /// `false` when the sender cannot send at all (e.g. `NOTIFY_BASE_URL` / `NOTIFY_API_KEY`
    /// unset). `my_profile`'s `emailCopy.available` reads it (W11).
    fn is_configured(&self) -> bool;

    /// Sends `message`. An unconfigured sender, a network failure or a non-2xx answer ⇒
    /// `AppError::Domain(EMAIL_UNAVAILABLE)`.
    fn send<'a>(&'a self, message: &'a EmailMessage) -> BoxFuture<'a, AppResult<EmailOutcome>>;
}
