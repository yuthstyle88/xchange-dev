//! APP-4 (#994) — the anonymous app session's id and its stored hash (design P7: an opaque random
//! id in the `xc_app_sid` cookie; only its SHA-256 is stored).
//! Written before the implementation by the ticket author's delegate (claude/dell-claude-3). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Signatures this test assumes (the scaffold commit before this one): `APP_SESSION_COOKIE`,
//! `AppSessionId::{generate, parse, as_str, hash}`, `SessionHash::as_str`. The id's format is not
//! pinned beyond "`parse` accepts what `generate` made".

use sha2::{Digest, Sha256};
use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSessionId};

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn the_cookie_is_named_xc_app_sid() {
    assert_eq!(APP_SESSION_COOKIE, "xc_app_sid");
}

#[test]
fn the_stored_hash_is_the_lower_hex_sha256_of_the_id() {
    let id = AppSessionId::generate();

    assert_eq!(id.hash().as_str(), sha256_hex(id.as_str()));
    assert_ne!(
        id.hash().as_str(),
        id.as_str(),
        "the id itself is never the stored value"
    );
}

#[test]
fn two_generated_ids_differ() {
    let a = AppSessionId::generate();
    let b = AppSessionId::generate();

    assert_ne!(a.as_str(), b.as_str());
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn a_generated_id_carries_at_least_128_bits() {
    let id = AppSessionId::generate();

    assert!(
        id.as_str().len() >= 22,
        "an id of {} characters is too short",
        id.as_str().len()
    );
}

#[test]
fn a_generated_id_parses_back_to_the_same_session() {
    let id = AppSessionId::generate();

    let parsed = AppSessionId::parse(id.as_str()).expect("an id generate made");

    assert_eq!(parsed.hash(), id.hash());
}

#[test]
fn an_empty_or_foreign_cookie_value_does_not_parse() {
    for raw in ["", " ", "not a session id!", "x"] {
        assert!(AppSessionId::parse(raw).is_none(), "{raw:?}");
    }
}

#[test]
fn the_debug_form_does_not_show_the_id() {
    let id = AppSessionId::generate();

    assert!(!format!("{id:?}").contains(id.as_str()));
}
