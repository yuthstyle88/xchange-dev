//! KWQ-D3 — `CopyByEmailService` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D21,
//! D22; §4.4 B19–B25; §4.5): the core of the e-mail copy, with a fake `EmailSender`, fake members
//! and fake subjects. **Both subjects run through the same table**, so a rule that holds for one and
//! not the other fails. The recipient is always the stored verified e-mail (the command has no
//! address); one idempotency key per QR; nothing is sent on any refusal.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use chrono::{DateTime, Duration, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use xc_customer::domain::email_sender::{EmailMessage, EmailOutcome, EmailSender};
use xc_customer::domain::sms_sender::BoxFuture;
use xchange::application::copy_by_email::link::CopyLink;
use xchange::application::copy_by_email::member_email::{MemberEmails, VerifiedEmail};
use xchange::application::copy_by_email::service::{
    CopyByEmailService, email_unavailable, link_expired, no_verified_email, not_found, qr_png,
};
use xchange::application::copy_by_email::subject::{CopyDetails, CopySubject, CopySubjectKind};

const ORIGIN: &str = "https://xc-v3.example";
const ALICE: &str = "CUST-ALICE";
const BOB: &str = "CUST-BOB";
const ALICE_MAIL: &str = "alice@example.com";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 22, 1, 0, 0).single().unwrap()
}

fn sha(qr: &str) -> [u8; 32] {
    Sha256::digest(qr.as_bytes()).into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ── fakes ───────────────────────────────────────────────────────────────────

/// One subject kind holding rows `(owner, id, current qr)`; records the hashes it was asked for.
struct FakeSubject {
    kind: CopySubjectKind,
    rows: Mutex<Vec<(String, String, String)>>,
    asked: Mutex<Vec<[u8; 32]>>,
}

impl FakeSubject {
    fn new(kind: CopySubjectKind, rows: &[(&str, &str, &str)]) -> Arc<Self> {
        Arc::new(Self {
            kind,
            rows: Mutex::new(
                rows.iter()
                    .map(|(o, i, q)| ((*o).to_owned(), (*i).to_owned(), (*q).to_owned()))
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn reissue(&self, id: &str, qr: &str) {
        for row in self.rows.lock().unwrap().iter_mut() {
            if row.1 == id {
                row.2 = qr.to_owned();
            }
        }
    }

    fn details(&self, id: &str) -> CopyDetails {
        CopyDetails {
            kind: self.kind,
            id: id.to_owned(),
            branch_name: "Silom".to_owned(),
            rows: vec![
                ("currency".to_owned(), "USD".to_owned()),
                ("amount".to_owned(), "100.00".to_owned()),
            ],
            status: "QUOTED".to_owned(),
        }
    }
}

impl CopySubject for FakeSubject {
    fn kind(&self) -> CopySubjectKind {
        self.kind
    }

    fn load_for<'a>(
        &'a self,
        member: &'a str,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        Box::pin(async move {
            self.asked.lock().unwrap().push(*qr_hash);
            let found = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .any(|(o, i, q)| o == member && i == id && &sha(q) == qr_hash);
            Ok(found.then(|| self.details(id)))
        })
    }

    fn load_by_link<'a>(
        &'a self,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        Box::pin(async move {
            let found = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .any(|(_, i, q)| i == id && &sha(q) == qr_hash);
            Ok(found.then(|| self.details(id)))
        })
    }
}

struct FakeMembers(BTreeMap<String, VerifiedEmail>);

impl MemberEmails for FakeMembers {
    fn verified_email<'a>(
        &'a self,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<VerifiedEmail>>> {
        Box::pin(async move { Ok(self.0.get(customer_id).cloned()) })
    }
}

/// Records every message; the first send of a key answers `Sent`, a repeat `AlreadySent`.
struct FakeSender {
    configured: bool,
    fail: bool,
    sent: Mutex<Vec<EmailMessage>>,
}

impl FakeSender {
    fn new(configured: bool, fail: bool) -> Arc<Self> {
        Arc::new(Self {
            configured,
            fail,
            sent: Mutex::new(Vec::new()),
        })
    }

    fn sent(&self) -> Vec<EmailMessage> {
        self.sent.lock().unwrap().clone()
    }
}

impl EmailSender for FakeSender {
    fn is_configured(&self) -> bool {
        self.configured
    }

    fn send<'a>(&'a self, message: &'a EmailMessage) -> BoxFuture<'a, AppResult<EmailOutcome>> {
        Box::pin(async move {
            if self.fail {
                return Err(email_unavailable());
            }
            let mut sent = self.sent.lock().unwrap();
            let again = sent
                .iter()
                .any(|m| m.idempotency_key == message.idempotency_key);
            sent.push(message.clone());
            Ok(if again {
                EmailOutcome::AlreadySent
            } else {
                EmailOutcome::Sent
            })
        })
    }
}

struct World {
    service: CopyByEmailService,
    booking: Arc<FakeSubject>,
    web: Arc<FakeSubject>,
    sender: Arc<FakeSender>,
}

fn world_with(sender: Arc<FakeSender>) -> World {
    let booking = FakeSubject::new(
        CopySubjectKind::Booking,
        &[(ALICE, "BK-1", "bk-qr-1"), (BOB, "BK-2", "bk-qr-2")],
    );
    let web = FakeSubject::new(
        CopySubjectKind::WebRequest,
        &[(ALICE, "WR-1", "wr-qr-1"), (BOB, "WR-2", "wr-qr-2")],
    );
    let members = Arc::new(FakeMembers(BTreeMap::from([(
        ALICE.to_owned(),
        VerifiedEmail {
            address: ALICE_MAIL.to_owned(),
            lang: "en".to_owned(),
        },
    )])));
    let service = CopyByEmailService::new(
        vec![booking.clone(), web.clone()],
        members,
        sender.clone(),
        CopyLink::new([9; 32]),
        ORIGIN.to_owned(),
    );
    World {
        service,
        booking,
        web,
        sender,
    }
}

fn world() -> World {
    world_with(FakeSender::new(true, false))
}

/// Both subjects: (kind, Alice's id, its qr, Bob's id, Bob's qr).
fn table() -> [(CopySubjectKind, &'static str, &'static str, &'static str, &'static str); 2] {
    [
        (CopySubjectKind::Booking, "BK-1", "bk-qr-1", "BK-2", "bk-qr-2"),
        (CopySubjectKind::WebRequest, "WR-1", "wr-qr-1", "WR-2", "wr-qr-2"),
    ]
}

fn same(a: &AppError, b: &AppError) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

// ── B19 / B20 ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_copy_goes_to_the_stored_address_with_one_key_per_qr_for_both_subjects() {
    for (kind, id, qr, _, _) in table() {
        let w = world();
        let first = w
            .service
            .send(ALICE, kind, id, qr, now())
            .await
            .unwrap_or_else(|e| panic!("{kind:?}: {e:?}"));
        assert!(!first.again, "{kind:?}");
        let sent = w.sender.sent();
        assert_eq!(sent.len(), 1, "{kind:?}: one Notify call");
        let m = &sent[0];
        assert_eq!(m.to, ALICE_MAIL, "{kind:?}: only the stored verified address");
        assert_eq!(
            m.idempotency_key,
            format!("copy:{}:{id}:{}", kind.as_str(), hex(&sha(qr))),
            "{kind:?}"
        );
        assert!(!m.subject.is_empty() && !m.text.is_empty(), "{kind:?}");
        assert!(
            m.html.contains("data:image/png;base64,"),
            "{kind:?}: the QR is inline: {}",
            m.html
        );
        assert!(m.text.contains("Silom") && m.html.contains("Silom"), "{kind:?}");
        assert!(m.text.contains("USD") && m.text.contains("100.00"), "{kind:?}");

        // the link in both bodies verifies and names this subject and QR, valid 24 h
        let prefix = format!("{ORIGIN}/c#");
        let start = m.text.find(&prefix).expect("a link in the text") + prefix.len();
        let link: String = m.text[start..]
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '<' && *c != '"')
            .collect();
        assert!(m.html.contains(&link), "{kind:?}: the same link in the HTML");
        let parts = CopyLink::new([9; 32])
            .verify(&link, now() + Duration::hours(23))
            .unwrap_or_else(|_| panic!("{kind:?}: the link verifies: {link}"));
        assert_eq!((parts.kind, parts.id.as_str(), parts.qr.as_str()), (kind, id, qr));
        assert!(
            CopyLink::new([9; 32])
                .verify(&link, now() + Duration::hours(25))
                .is_err(),
            "{kind:?}: one day only"
        );

        // B20: the same QR again ⇒ the same key, `again`
        let second = w.service.send(ALICE, kind, id, qr, now()).await.expect("again");
        assert!(second.again, "{kind:?}");
        let keys: Vec<String> = w.sender.sent().iter().map(|m| m.idempotency_key.clone()).collect();
        assert_eq!(keys[0], keys[1], "{kind:?}");
    }
}

/// D3, D22: the QR in the e-mail must work at the queue machine and the desk exactly as on the
/// phone. A web-request QR therefore encodes `<origin>/q#<qr>`, the only form the queue machine
/// (F4, A7) and the desk (F13) read; a booking QR stays the raw token. Added by the slice
/// controller (claude/mbp-xc-ks) after the PR 4488 review (issuecomment-6328924380, must-fix 3).
#[tokio::test]
async fn the_email_qr_encodes_what_the_queue_machine_and_the_desk_read() {
    for (kind, id, qr, _, _) in table() {
        let w = world();
        w.service
            .send(ALICE, kind, id, qr, now())
            .await
            .unwrap_or_else(|e| panic!("{kind:?}: {e:?}"));
        let encoded = match kind {
            CopySubjectKind::WebRequest => format!("{ORIGIN}/q#{qr}"),
            CopySubjectKind::Booking => qr.to_owned(),
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(qr_png(&encoded));
        let sent = w.sender.sent();
        assert!(
            sent[0].html.contains(&format!("data:image/png;base64,{b64}\"")),
            "{kind:?}: the inline QR encodes {encoded}"
        );
    }
}

#[tokio::test]
async fn the_subject_is_asked_with_the_sha256_of_the_qr_never_the_qr() {
    let w = world();
    w.service
        .send(ALICE, CopySubjectKind::Booking, "BK-1", "bk-qr-1", now())
        .await
        .expect("sent");
    assert_eq!(w.booking.asked.lock().unwrap().as_slice(), &[sha("bk-qr-1")]);
    assert!(w.web.asked.lock().unwrap().is_empty(), "only the named subject");
}

// ── B21 / B22 / B23 ─────────────────────────────────────────────────────────

#[tokio::test]
async fn another_members_id_an_unknown_id_or_a_reissued_qr_is_not_found_and_sends_nothing() {
    for (kind, id, qr, bobs_id, bobs_qr) in table() {
        let w = world();
        for (who, which, code) in [
            (ALICE, bobs_id, bobs_qr),
            (ALICE, "no-such", qr),
            (ALICE, id, "wrong-qr"),
        ] {
            let err = w.service.send(who, kind, which, code, now()).await.unwrap_err();
            assert!(same(&err, &not_found()), "{kind:?} {which}: {err:?}");
        }
        let subject = if kind == CopySubjectKind::Booking { &w.booking } else { &w.web };
        subject.reissue(id, "fresh-qr");
        let err = w.service.send(ALICE, kind, id, qr, now()).await.unwrap_err();
        assert!(same(&err, &not_found()), "{kind:?} reissued-away: {err:?}");
        assert!(w.sender.sent().is_empty(), "{kind:?}: nothing sent");
    }
}

#[tokio::test]
async fn a_member_with_no_verified_email_is_refused_and_nothing_is_sent() {
    for (kind, _, _, bobs_id, bobs_qr) in table() {
        let w = world();
        let err = w.service.send(BOB, kind, bobs_id, bobs_qr, now()).await.unwrap_err();
        assert!(same(&err, &no_verified_email()), "{kind:?}: {err:?}");
        assert!(w.sender.sent().is_empty(), "{kind:?}");
    }
}

#[tokio::test]
async fn an_unconfigured_or_failing_sender_is_email_unavailable() {
    for (kind, id, qr, _, _) in table() {
        let off = world_with(FakeSender::new(false, false));
        let err = off.service.send(ALICE, kind, id, qr, now()).await.unwrap_err();
        assert!(same(&err, &email_unavailable()), "{kind:?}: {err:?}");
        assert!(off.sender.sent().is_empty(), "{kind:?}");

        let failing = world_with(FakeSender::new(true, true));
        let err = failing.service.send(ALICE, kind, id, qr, now()).await.unwrap_err();
        assert!(same(&err, &email_unavailable()), "{kind:?}: {err:?}");
    }
}

#[tokio::test]
async fn the_checks_run_subject_first_then_email_then_sender() {
    // an unconfigured sender still answers notFound for a wrong id, and noVerifiedEmail for Bob
    let w = world_with(FakeSender::new(false, false));
    let err = w
        .service
        .send(ALICE, CopySubjectKind::Booking, "no-such", "bk-qr-1", now())
        .await
        .unwrap_err();
    assert!(same(&err, &not_found()), "{err:?}");
    let err = w
        .service
        .send(BOB, CopySubjectKind::Booking, "BK-2", "bk-qr-2", now())
        .await
        .unwrap_err();
    assert!(same(&err, &no_verified_email()), "{err:?}");
}

// ── B24 / B25 ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_good_link_shows_the_details_and_every_failure_is_link_expired() {
    for (kind, id, qr, _, _) in table() {
        let w = world();
        let link = CopyLink::new([9; 32]).sign(kind, id, qr, now() + Duration::hours(24));
        let view = w
            .service
            .view_link(&link, now() + Duration::hours(23))
            .await
            .unwrap_or_else(|e| panic!("{kind:?}: {e:?}"));
        assert_eq!(view.details.id, id);
        assert_eq!(view.details.kind, kind);
        assert_eq!(view.qr, qr);
        assert_eq!(view.expires_at, now() + Duration::hours(24));

        let expired = w.service.view_link(&link, now() + Duration::hours(25)).await.unwrap_err();
        assert!(same(&expired, &link_expired()), "{kind:?}: {expired:?}");
        let forged = CopyLink::new([1; 32]).sign(kind, id, qr, now() + Duration::hours(24));
        let err = w.service.view_link(&forged, now()).await.unwrap_err();
        assert!(same(&err, &link_expired()), "{kind:?}: {err:?}");
        let err = w.service.view_link("garbage", now()).await.unwrap_err();
        assert!(same(&err, &link_expired()), "{kind:?}: {err:?}");

        let subject = if kind == CopySubjectKind::Booking { &w.booking } else { &w.web };
        subject.reissue(id, "fresh-qr");
        let err = w.service.view_link(&link, now()).await.unwrap_err();
        assert!(same(&err, &link_expired()), "{kind:?} reissued: {err:?}");
        assert!(w.sender.sent().is_empty(), "viewing sends nothing");
    }
}

// ── the QR picture ──────────────────────────────────────────────────────────

#[test]
fn the_qr_picture_is_a_png_and_the_same_data_gives_the_same_bytes() {
    let png = qr_png("https://xc-v3.example/q#4f1c2a9e");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(png, qr_png("https://xc-v3.example/q#4f1c2a9e"));
    assert_ne!(png, qr_png("https://xc-v3.example/q#other"));
    // what the HTML embeds decodes back to these bytes
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    assert_eq!(
        base64::engine::general_purpose::STANDARD.decode(b64).unwrap(),
        png
    );
}
