//! RBAC C1a up-front tests (design `docs/design/blueprint/user-management-rbac.md`, PR 3083, §3.4,
//! owner decision Q5 A; V.9 "Temporary passwords"): `TempPassword::generate()` gives 16
//! characters with no look-alike characters, different every time, and never written to a log.
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! The look-alike set below is the ticket's proposal (`LOOK_ALIKES`); the design names none (C1a
//! question 3). Names the new type, so on `origin/main` this file fails to compile; that is its
//! red. No database.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, OnceLock};

use xc_auth::domain::user::TempPassword;

/// Characters a person misreads for another: zero/O/o, one/l/I.
const LOOK_ALIKES: &[char] = &['0', 'O', 'o', '1', 'l', 'I'];

#[test]
fn a_temporary_password_has_sixteen_characters() {
    let _ = log();
    for _ in 0..500 {
        let p = TempPassword::generate();
        assert_eq!(
            p.expose().chars().count(),
            16,
            "{:?} chars",
            p.expose().len()
        );
        assert_eq!(p.expose().len(), 16, "one byte per character (ASCII)");
    }
}

#[test]
fn a_temporary_password_has_no_look_alike_or_blank_character() {
    let _ = log();
    for _ in 0..2000 {
        let p = TempPassword::generate();
        for c in p.expose().chars() {
            assert!(!LOOK_ALIKES.contains(&c), "look-alike {c:?}");
            assert!(
                c.is_ascii_graphic(),
                "not a printable ASCII character: {c:?}"
            );
        }
    }
}

#[test]
fn every_temporary_password_is_new() {
    let _ = log();
    let all: HashSet<String> = (0..1000)
        .map(|_| TempPassword::generate().expose().to_owned())
        .collect();
    assert_eq!(all.len(), 1000, "two of 1000 were the same");
}

#[test]
fn many_characters_are_used() {
    let _ = log();
    let used: HashSet<char> = (0..2000)
        .flat_map(|_| {
            TempPassword::generate()
                .expose()
                .chars()
                .collect::<Vec<_>>()
        })
        .collect();
    // 32 symbols x 16 characters = 80 bits at least
    assert!(
        used.len() >= 32,
        "only {} distinct characters in 32000",
        used.len()
    );
}

#[test]
fn debug_does_not_show_the_password() {
    let _ = log();
    let p = TempPassword::generate();
    let shown = format!("{p:?} {p:#?}");
    assert!(!shown.contains(p.expose()), "Debug shows it: {shown}");
}

// ---------------------------------------------------------------- the log

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

struct Text(String);

impl tracing::field::Visit for Text {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let _ = write!(self.0, "{}={} ", field.name(), value);
    }
}

impl Log {
    fn push(&self, text: String) {
        self.0.lock().expect("log lock").push(text);
    }
    fn all(&self) -> String {
        self.0.lock().expect("log lock").join("\n")
    }
}

impl tracing::Subscriber for Log {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut text = Text(String::new());
        attrs.record(&mut text);
        self.push(format!("span {} {}", attrs.metadata().name(), text.0));
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut text = Text(String::new());
        values.record(&mut text);
        self.push(text.0);
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = Text(String::new());
        event.record(&mut text);
        self.push(format!("{} {}", event.metadata().target(), text.0));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// The process-wide capturing subscriber, installed by the first test that runs. Global, not
/// `with_default`: a thread-local default misses events whose callsite another test thread
/// registered first (measured: an `info!` of the password passed under `with_default` with
/// parallel tests, and failed with `--test-threads=1`).
fn log() -> &'static Log {
    static LOG: OnceLock<Log> = OnceLock::new();
    LOG.get_or_init(|| {
        let log = Log::default();
        tracing::subscriber::set_global_default(log.clone())
            .expect("no other global subscriber in this test binary");
        log
    })
}

#[test]
fn generating_writes_no_password_to_the_log() {
    let log = log();
    // proves the capture works: this line must be in the log
    tracing::info!(probe = "rbac-c1a", "capture is on");
    let made: Vec<String> = (0..50)
        .map(|_| TempPassword::generate().expose().to_owned())
        .collect();
    let text = log.all();
    assert!(text.contains("rbac-c1a"), "the capture saw nothing: {text}");
    for p in &made {
        assert!(
            !text.contains(p.as_str()),
            "the log holds a generated password"
        );
    }
}
