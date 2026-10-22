//! `CopySubject` (design `kiosk-shared-phone.md` §4.2 D21): one small implementation per subject
//! the member can have e-mailed. A third subject later is one file.

use pos_error::AppResult;
use xc_customer::domain::sms_sender::BoxFuture;

/// The subjects, as named on the wire (`copyByEmail {subject}`) and in the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CopySubjectKind {
    Booking,
    WebRequest,
}

impl CopySubjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Booking => "booking",
            Self::WebRequest => "webRequest",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "booking" => Some(Self::Booking),
            "webRequest" => Some(Self::WebRequest),
            _ => None,
        }
    }
}

/// What the e-mail and the link page show. Never a passport, a phone or a customer number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyDetails {
    pub kind: CopySubjectKind,
    /// `bookings.id` / `kiosk_requests.id`.
    pub id: String,
    pub branch_name: String,
    /// `[proposed]` ordered `(field, value)` rows. Booking: `bookingNo`, `date`, `slot`,
    /// `currency`, `amount`. Web request: `currency`, `amount`, `rate`, `expiresAt`.
    pub rows: Vec<(String, String)>,
    /// The live status: the booking's own status, or a web request's W5 status.
    pub status: String,
}

pub trait CopySubject: Send + Sync {
    fn kind(&self) -> CopySubjectKind;

    /// The member's own subject `id` whose stored QR hash equals `qr_hash` (SHA-256 of the QR):
    /// another member's, an unknown id or a reissued-away QR ⇒ `Ok(None)`.
    fn load_for<'a>(
        &'a self,
        member: &'a str,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>>;

    /// The same for the public link (no member): by `id` and `qr_hash` only.
    fn load_by_link<'a>(
        &'a self,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>>;
}
