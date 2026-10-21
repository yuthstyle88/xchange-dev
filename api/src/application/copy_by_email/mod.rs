//! The e-mail copy of a booking or a web request (design `kiosk-shared-phone.md` §4.2 D21, D22).
//!
//! One core (`CopyByEmailService`) holds every rule; a `CopySubject` per subject only loads its
//! own row (♻️). The safety rules — the recipient is the member's stored verified e-mail, the
//! idempotency key, the signed link and its single `410` — live in the core, never in a subject.
//!
//! Amendment to §4.3 (slice d skeleton, claude/mbp-xc-ks): `BookingCopy` and `WebRequestCopy` live
//! here, beside the trait, not in `xc-booking` / `xc-kiosk`: a context crate cannot implement a
//! trait of the root package, and each subject is one SQL read.

pub mod booking_copy;
pub mod link;
pub mod member_email;
pub mod service;
pub mod subject;
pub mod web_request_copy;
