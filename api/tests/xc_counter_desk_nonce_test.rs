//! XC Counter, ticket CA-1b (`docs/design/xc-counter-app.md` §5.4, §8.3): the desk nonce. No database. Written
//! before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! | #  | Case | Expected |
//! |----|------|----------|
//! | N1 | `issue(agent, branch, now)` | `exp` = `now` + 60 s, in unix seconds |
//! | N2 | `seal` | `v1.<agentId>.<b64url(branchId)>.<exp>.<b64url(mac)>`, five parts, deterministic |
//! | N3 | `open` of a sealed nonce, same keys, `now < exp` | the same nonce |
//! | N4 | `open` at `now == exp` or later | `Err(Expired)` |
//! | N5 | another key file | `Err(Invalid)` |
//! | N6 | any one part changed (version, agent, branch, exp, mac), a part added or dropped, bad base64, empty | `Err(Invalid)` |
//! | N7 | the MAC is keyed by `derive_subkey(keys, "xc-desk-binding-v1")`, so it differs from the OTP seal's subkey | a value whose MAC uses the raw key or another info string is `Err(Invalid)` |
//! | N8 | refusal codes | `NONCE_INVALID`, `NONCE_EXPIRED` |

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, TimeZone, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use xc_kernel::pii::PiiKeys;
use xchange::infrastructure::device_agents::desk_binding::{
    DeskNonce, NonceRefusal, SUBKEY_INFO, TTL_SECS,
};

const AGENT: &str = "c0000000-0000-4000-8000-00000000000a";
const BRANCH: &str = "MAIN";

fn keys() -> PiiKeys {
    PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").expect("valid pii keys")
}

fn other_keys() -> PiiKeys {
    PiiKeys::parse("test-2:HyAhIiMkJSYnKCkqKywtLi8wMTIzNDU2Nzg5Ojs8PT4=").expect("valid pii keys")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 6, 3, 0, 0)
        .single()
        .expect("an instant")
}

/// The MAC as §8.3 defines it, over the given subkey.
fn mac_with(subkey: &[u8], agent: &str, branch: &str, exp: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(subkey).expect("any key length");
    for (i, part) in ["v1", agent, branch, &exp.to_string()].iter().enumerate() {
        if i > 0 {
            mac.update(&[0]);
        }
        mac.update(part.as_bytes());
    }
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

fn wire(agent: &str, branch: &str, exp: i64, mac: &str) -> String {
    format!("v1.{agent}.{}.{exp}.{mac}", URL_SAFE_NO_PAD.encode(branch))
}

#[test]
fn the_constants_are_the_design_s() {
    assert_eq!(SUBKEY_INFO, b"xc-desk-binding-v1");
    assert_eq!(TTL_SECS, 60);
    assert_eq!(NonceRefusal::Invalid.code(), "NONCE_INVALID", "N8");
    assert_eq!(NonceRefusal::Expired.code(), "NONCE_EXPIRED", "N8");
}

#[test]
fn issue_seal_and_open_round_trip() {
    let nonce = DeskNonce::issue(AGENT, BRANCH, now());
    assert_eq!(
        nonce,
        DeskNonce {
            agent_id: AGENT.to_owned(),
            branch_id: BRANCH.to_owned(),
            exp: now().timestamp() + 60,
        },
        "N1"
    );
    let sealed = nonce.seal(&keys());
    assert_eq!(sealed, nonce.seal(&keys()), "N2: deterministic");
    let subkey = xc_kernel::pii::derive_subkey(&keys(), b"xc-desk-binding-v1");
    assert_eq!(
        sealed,
        wire(
            AGENT,
            BRANCH,
            nonce.exp,
            &mac_with(&subkey, AGENT, BRANCH, nonce.exp)
        ),
        "N2: the wire form"
    );
    assert_eq!(
        DeskNonce::open(&sealed, &keys(), now()),
        Ok(nonce.clone()),
        "N3"
    );
    assert_eq!(
        DeskNonce::open(&sealed, &keys(), now() + Duration::seconds(59)),
        Ok(nonce),
        "N3: one second before exp"
    );
}

#[test]
fn an_expired_nonce_is_refused() {
    let sealed = DeskNonce::issue(AGENT, BRANCH, now()).seal(&keys());
    assert_eq!(
        DeskNonce::open(&sealed, &keys(), now() + Duration::seconds(60)),
        Err(NonceRefusal::Expired),
        "N4: at exp"
    );
    assert_eq!(
        DeskNonce::open(&sealed, &keys(), now() + Duration::hours(1)),
        Err(NonceRefusal::Expired),
        "N4"
    );
}

#[test]
fn anything_but_a_sealed_nonce_is_invalid() {
    let nonce = DeskNonce::issue(AGENT, BRANCH, now());
    let sealed = nonce.seal(&keys());
    assert_eq!(
        DeskNonce::open(&sealed, &other_keys(), now()),
        Err(NonceRefusal::Invalid),
        "N5"
    );

    let parts: Vec<&str> = sealed.split('.').collect();
    assert_eq!(parts.len(), 5, "N2");
    let other_branch = URL_SAFE_NO_PAD.encode("B9");
    let later = (nonce.exp + 3600).to_string();
    let mut bad: Vec<String> = vec![
        String::new(),
        "v1".to_owned(),
        format!("{sealed}.extra"),
        parts[..4].join("."),
        sealed.replacen("v1.", "v2.", 1),
        sealed.replacen(AGENT, "c0000000-0000-4000-8000-00000000000b", 1),
        [parts[0], parts[1], &other_branch, parts[3], parts[4]].join("."),
        [parts[0], parts[1], parts[2], &later, parts[4]].join("."),
        [parts[0], parts[1], parts[2], parts[3], "AAAA"].join("."),
        [parts[0], parts[1], "***", parts[3], parts[4]].join("."),
        [parts[0], parts[1], parts[2], "soon", parts[4]].join("."),
    ];
    let mut flipped = parts[4].to_owned().into_bytes();
    flipped[0] = if flipped[0] == b'A' { b'B' } else { b'A' };
    bad.push(
        [
            parts[0],
            parts[1],
            parts[2],
            parts[3],
            &String::from_utf8(flipped).expect("ascii"),
        ]
        .join("."),
    );
    for value in &bad {
        assert_eq!(
            DeskNonce::open(value, &keys(), now()),
            Err(NonceRefusal::Invalid),
            "N6: {value:?}"
        );
    }

    let raw = mac_with(b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f", AGENT, BRANCH, nonce.exp);
    let otp = mac_with(
        &xc_kernel::pii::derive_subkey(&keys(), b"xc-otp-challenge-v1"),
        AGENT,
        BRANCH,
        nonce.exp,
    );
    for mac in [raw, otp] {
        assert_eq!(
            DeskNonce::open(&wire(AGENT, BRANCH, nonce.exp, &mac), &keys(), now()),
            Err(NonceRefusal::Invalid),
            "N7"
        );
    }
}
