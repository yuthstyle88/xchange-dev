//! #1027 W2-T5 (#4049) — tests of `client_address` itself, restored verbatim from
//! `api/tests/client_address_test.rs` (`2982a779^:api/tests/client_address_test.rs`, lines 298–466).

use std::path::{Path, PathBuf};

use actix_web::http::header::HeaderValue;
use actix_web::test::TestRequest;
use xchange::presentation::http::client_address::{
    CLIENT_ADDRESS_HEADER, UNKNOWN_ADDRESS, client_address,
};

const CF: &str = "cf-connecting-ip";
const XFF: &str = "x-forwarded-for";
const REAL: &str = "x-real-ip";

// ── case 5: the function ─────────────────────────────────────────────────

fn address_of(request: TestRequest) -> String {
    client_address(&request.to_http_request()).to_owned()
}

#[test]
fn the_header_and_the_address_of_nobody_are_these() {
    assert_eq!(CLIENT_ADDRESS_HEADER, "x-real-ip");
    assert_eq!(UNKNOWN_ADDRESS, "unknown");
}

#[test]
fn the_address_is_what_x_real_ip_says() {
    assert_eq!(
        address_of(TestRequest::default().insert_header((REAL, "203.0.113.7"))),
        "203.0.113.7"
    );
    assert_eq!(
        address_of(TestRequest::default().insert_header(("X-Real-IP", "2001:db8::1"))),
        "2001:db8::1",
        "header names are not case-sensitive"
    );
}

#[test]
fn spaces_around_the_address_are_not_part_of_it() {
    for sent in [
        " 203.0.113.7",
        "203.0.113.7 ",
        "\t203.0.113.7\t ",
        "  203.0.113.7  ",
    ] {
        assert_eq!(
            address_of(TestRequest::default().insert_header((REAL, sent))),
            "203.0.113.7",
            "{sent:?}"
        );
    }
}

#[test]
fn a_request_without_the_header_is_from_unknown() {
    assert_eq!(address_of(TestRequest::default()), "unknown");
}

#[test]
fn an_empty_or_blank_header_is_from_unknown() {
    for sent in ["", " ", "   ", "\t", " \t "] {
        assert_eq!(
            address_of(TestRequest::default().insert_header((REAL, sent))),
            "unknown",
            "{sent:?}"
        );
    }
}

#[test]
fn a_header_that_is_not_text_is_from_unknown_and_nothing_panics() {
    let not_text: [&[u8]; 4] = [
        &[0xff, 0xfe],
        &[b'1', b'.', b'2', 0x80, b'3'],
        "ที่อยู่".as_bytes(),
        &[0xc3, 0x28],
    ];
    for bytes in not_text {
        let value = HeaderValue::from_bytes(bytes).expect("a header value may hold these bytes");
        assert_eq!(
            address_of(TestRequest::default().insert_header((REAL, value))),
            "unknown",
            "{bytes:?}"
        );
    }
}

#[test]
fn the_headers_a_client_can_write_are_never_the_address() {
    assert_eq!(
        address_of(TestRequest::default().insert_header((CF, "198.51.100.1"))),
        "unknown"
    );
    assert_eq!(
        address_of(TestRequest::default().insert_header((XFF, "198.51.100.2"))),
        "unknown"
    );
    assert_eq!(
        address_of(
            TestRequest::default()
                .insert_header((CF, "198.51.100.1"))
                .insert_header((XFF, "198.51.100.2"))
                .insert_header((REAL, "203.0.113.7"))
        ),
        "203.0.113.7"
    );
    assert_eq!(
        address_of(
            TestRequest::default()
                .insert_header((CF, "198.51.100.1"))
                .insert_header((XFF, "198.51.100.2"))
                .insert_header((REAL, ""))
        ),
        "unknown",
        "an empty X-Real-IP does not fall back to a header the client wrote"
    );
}

// ── one place ────────────────────────────────────────────────────────────

fn rust_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|name| name != "target" && name != "tests")
            {
                rust_sources(&path, found);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// The address is read in one place: no other server source names an address header.
/// A later per-address limit calls `client_address`; it does not read the request itself.
#[test]
fn no_server_source_but_client_address_names_an_address_header() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = Vec::new();
    rust_sources(&root.join("src"), &mut sources);
    rust_sources(&root.join("crates"), &mut sources);
    assert!(
        sources.len() > 20,
        "the walk found the server's sources: {}",
        sources.len()
    );
    let the_one_place = root.join("src/presentation/http/client_address.rs");
    assert!(
        sources.contains(&the_one_place),
        "{}",
        the_one_place.display()
    );

    let mut offences = Vec::new();
    for path in &sources {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
            .to_lowercase();
        for (number, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for header in ["cf-connecting-ip", "x-forwarded-for", "x-real-ip"] {
                let allowed = header == "x-real-ip" && *path == the_one_place;
                if code.contains(header) && !allowed {
                    offences.push(format!("{}:{}: {header}", path.display(), number + 1));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "an address header is named outside client_address.rs:\n{}",
        offences.join("\n")
    );
}
