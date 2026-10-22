//! KWQ-C2-3 — `OtpChallengeSeal` (design `kiosk-shared-phone.md` §4.2 D11; the seal's wire form is the
//! `[required]` header of `api/src/application/otp_challenge_seal.rs`): a seal opens to exactly what was sealed,
//! and any change, another key or a foreign value opens to nothing.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit, delete or weaken;
//! add tests in a file of your own.

use xc_kernel::pii::PiiKeys;
use xchange::application::otp_challenge_seal::{OtpChallengeSeal, OtpChannel};

fn keys() -> PiiKeys {
    PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").expect("a test key")
}

fn other_keys() -> PiiKeys {
    PiiKeys::parse("test-2:HyAhIiMkJSYnKCkqKywtLi8wMTIzNDU2Nzg5Ojs8PT4=").expect("another key")
}

fn email() -> OtpChallengeSeal {
    OtpChallengeSeal {
        channel: OtpChannel::Email,
        destination: "somchai@example.com".to_owned(),
        challenge_id: "c0ffee00-0000-4000-8000-000000000001".to_owned(),
    }
}

#[test]
fn a_seal_opens_to_what_was_sealed_for_both_channels() {
    let k = keys();
    let sms = OtpChallengeSeal {
        channel: OtpChannel::Sms,
        destination: "+66812345678".to_owned(),
        challenge_id: "c0ffee00-0000-4000-8000-000000000002".to_owned(),
    };
    for seal in [email(), sms] {
        let value = seal.seal(&k);
        assert!(value.starts_with("v1."), "{value}");
        assert_eq!(value.split('.').count(), 5, "{value}");
        assert!(!value.contains("somchai@"), "the destination is not in the clear: {value}");
        assert_eq!(OtpChallengeSeal::open(&value, &k), Some(seal.clone()));
        assert_eq!(seal.seal(&k), value, "deterministic");
    }
}

#[test]
fn any_changed_character_opens_to_nothing() {
    let k = keys();
    let value = email().seal(&k);
    for i in 0..value.len() {
        let mut bytes = value.clone().into_bytes();
        bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
        let changed = String::from_utf8(bytes).unwrap();
        if changed == value {
            continue;
        }
        assert_eq!(
            OtpChallengeSeal::open(&changed, &k),
            None,
            "position {i}: {changed}"
        );
    }
}

#[test]
fn another_key_a_foreign_value_and_a_swapped_channel_open_to_nothing() {
    let value = email().seal(&keys());
    assert_eq!(OtpChallengeSeal::open(&value, &other_keys()), None);
    for foreign in ["", "v1", "v1....", "v2.email.x.y.z", "garbage"] {
        assert_eq!(OtpChallengeSeal::open(foreign, &keys()), None, "{foreign:?}");
    }
    let swapped = value.replacen("v1.email.", "v1.sms.", 1);
    assert_eq!(OtpChallengeSeal::open(&swapped, &keys()), None);
}

#[test]
fn channels_parse_and_print() {
    assert_eq!(OtpChannel::parse("sms"), Some(OtpChannel::Sms));
    assert_eq!(OtpChannel::parse("email"), Some(OtpChannel::Email));
    assert_eq!(OtpChannel::parse("SMS"), None);
    assert_eq!(OtpChannel::Email.as_str(), "email");
}
