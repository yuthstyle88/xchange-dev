//! KWQ-D2 — `CopyLink` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D22, §4.4 B24,
//! B25): the signed, unstored 1-day link. Sign → verify round trips; a link with any one character
//! changed, a link 25 h old, another key's link and a malformed one are all rejected the same way.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xchange::application::copy_by_email::link::{CopyLink, LinkParts, LinkRejected};
use xchange::application::copy_by_email::subject::CopySubjectKind;

fn sent() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 22, 1, 0, 0).single().unwrap()
}

fn key(byte: u8) -> CopyLink {
    CopyLink::new([byte; 32])
}

const QR: &str = "4f1c2a9e-0d3b-4c7a-9e21-7b5d3c1a0f88";

fn link() -> String {
    key(7).sign(
        CopySubjectKind::WebRequest,
        "6b9c1f2e-1111-4222-8333-944455556666",
        QR,
        sent() + Duration::hours(24),
    )
}

#[test]
fn a_signed_link_verifies_and_names_what_was_signed() {
    let parts = key(7)
        .verify(&link(), sent() + Duration::hours(23))
        .expect("a 23 h old link is good");
    assert_eq!(
        parts,
        LinkParts {
            kind: CopySubjectKind::WebRequest,
            id: "6b9c1f2e-1111-4222-8333-944455556666".to_owned(),
            qr: QR.to_owned(),
            expires_at: sent() + Duration::hours(24),
        }
    );
    assert!(link().starts_with("v1."), "versioned: {}", link());
}

#[test]
fn both_subjects_round_trip_including_a_booking_qr() {
    let qr = "Zm9vYmFyYmF6cXV4LWJvb2tpbmctcXItdG9rZW4tMDAx"; // base64url, as the booking QR
    let l = key(7).sign(
        CopySubjectKind::Booking,
        "BK-0b6e8d3c-2f1a-4e5b-9c7d-112233445566",
        qr,
        sent() + Duration::hours(24),
    );
    let parts = key(7).verify(&l, sent()).expect("good");
    assert_eq!(parts.kind, CopySubjectKind::Booking);
    assert_eq!(parts.id, "BK-0b6e8d3c-2f1a-4e5b-9c7d-112233445566");
    assert_eq!(parts.qr, qr);
}

#[test]
fn a_link_past_its_day_is_rejected() {
    assert_eq!(
        key(7).verify(&link(), sent() + Duration::hours(25)),
        Err(LinkRejected)
    );
    assert_eq!(
        key(7).verify(&link(), sent() + Duration::hours(24)),
        Err(LinkRejected),
        "expires_at itself is no longer valid (exp > now)"
    );
}

#[test]
fn any_one_character_changed_is_rejected() {
    let good = link();
    let now = sent() + Duration::hours(1);
    for i in 0..good.len() {
        let mut bytes = good.clone().into_bytes();
        bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
        let changed = String::from_utf8(bytes).unwrap();
        if changed == good {
            continue;
        }
        assert_eq!(
            key(7).verify(&changed, now),
            Err(LinkRejected),
            "character {i} changed: {changed}"
        );
    }
}

#[test]
fn another_keys_link_and_malformed_links_are_rejected() {
    let now = sent() + Duration::hours(1);
    assert_eq!(key(8).verify(&link(), now), Err(LinkRejected));
    for bad in [
        "",
        "v1",
        "v1.....",
        "v2.webRequest.x.1.y.z",
        "garbage",
        "v1.unknownSubject.6b9c.1792630800.qr.mac",
    ] {
        assert_eq!(key(7).verify(bad, now), Err(LinkRejected), "{bad:?}");
    }
    // a link with a part appended or cut off
    assert_eq!(key(7).verify(&format!("{}x", link()), now), Err(LinkRejected));
    let cut = &link()[..link().len() - 1];
    assert_eq!(key(7).verify(cut, now), Err(LinkRejected));
}

#[test]
fn the_url_puts_the_link_in_the_fragment() {
    assert_eq!(
        CopyLink::url("https://xc-v3.example/", "v1.a.b"),
        "https://xc-v3.example/c#v1.a.b"
    );
}
