//! Up-front tests for FD-3 (#974): `SystemToken`, the `X-System-Token` check of the Broker feed
//! route, without a server. Written by the ticket author before the implementation. Do not edit,
//! delete or weaken; add tests in a file of your own.

use xchange::presentation::http::v3::system::token::SystemToken;

const TOKEN: &str = "fd3-feed-token-0123456789abcdef";

#[test]
fn the_configured_token_is_accepted_and_nothing_else() {
    let t = SystemToken::from_value(Some(TOKEN.to_string()));
    assert!(t.accepts(Some(TOKEN.as_bytes())));
    for wrong in [
        "",
        "fd3",
        "fd3-feed-token-0123456789abcde",
        "fd3-feed-token-0123456789abcdef0",
        "fd3-feed-token-0123456789abcdeF",
        " fd3-feed-token-0123456789abcdef",
        "fd3-feed-token-0123456789abcdef ",
    ] {
        assert!(
            !t.accepts(Some(wrong.as_bytes())),
            "{wrong:?} must be refused"
        );
    }
    assert!(!t.accepts(None), "no header must be refused");
}

#[test]
fn an_unset_token_accepts_nothing() {
    let t = SystemToken::from_value(None);
    for presented in [Some(TOKEN.as_bytes()), Some(&b""[..]), None] {
        assert!(!t.accepts(presented), "{presented:?}");
    }
}

#[test]
fn an_empty_or_blank_token_accepts_nothing_not_even_itself() {
    for blank in ["", " ", "   ", "\t", "\n"] {
        let t = SystemToken::from_value(Some(blank.to_string()));
        assert!(!t.accepts(Some(blank.as_bytes())), "{blank:?} configured");
        assert!(!t.accepts(Some(b"")), "{blank:?} configured, empty sent");
        assert!(!t.accepts(None), "{blank:?} configured, nothing sent");
    }
}
