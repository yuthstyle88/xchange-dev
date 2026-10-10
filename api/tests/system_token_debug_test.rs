//! Up-front tests for #1112: `SystemToken`'s `Debug` never shows the token. Written by the ticket
//! author (claude/5d45b017, on the xchange lane controller's order) before the implementation. Do
//! not edit, delete or weaken; add tests in a file of your own.
//!
//! #1112 §3: a hand-written `impl fmt::Debug for SystemToken` that prints only whether a token is
//! configured, `SystemToken { configured: true }`; unset (and, by `from_value`, empty or blank)
//! prints `configured: false`. The derived `Debug` printed the token's bytes. Needs no database.

use xchange::presentation::http::v3::system::token::SystemToken;

const SECRET: &str = "s3cret";

#[test]
fn a_configured_token_never_appears_in_debug() {
    let t = SystemToken::from_value(Some(SECRET.to_string()));
    let shown = format!("{t:?}");
    assert!(!shown.contains(SECRET), "Debug leaks the token: {shown}");
    // The bytes as the derive printed them: [115, 51, 99, 114, 101, 116].
    let bytes = format!("{:?}", SECRET.as_bytes());
    let bytes_inner = bytes.trim_start_matches('[').trim_end_matches(']');
    assert!(
        !shown.contains(bytes_inner),
        "Debug leaks the token's bytes: {shown}"
    );
    assert_eq!(shown, "SystemToken { configured: true }");
    // Pretty Debug ({:#?}) must not leak it either.
    assert!(!format!("{t:#?}").contains(SECRET));
}

#[test]
fn an_unset_empty_or_blank_token_debugs_as_not_configured() {
    for value in [None, Some(String::new()), Some("   ".to_string())] {
        let t = SystemToken::from_value(value.clone());
        assert_eq!(
            format!("{t:?}"),
            "SystemToken { configured: false }",
            "{value:?}"
        );
    }
}
