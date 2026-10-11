//! Ticket #759 (RE-4e) — for the harnesses of RE-4e's test files that set their counter rate:
//! publish that rate, so that it is the counter rate once the desk save, the kiosk and bookings
//! read the current publication (RE-4b…d).
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Re-expected for RE-7 #604 by the ticket author: migration 6 drops `rates`, so the harnesses no
//! longer append a row and publish "the branch's newest rows" (`publish_newest_rates`, removed).
//! They state the rate they set: `set_counter_rate` takes it per 1, as the retired direct save
//! took it, and publishes it × `currencies.unit_size` over the branch's current publication —
//! the same publication the newest rows made, since every harness rate is set this way.
//!
//! Design: `docs/design/blueprint/rates.md` D-3 and §4.8 (the current publication is the counter
//! rate; every quoting reader reads `PublishedRates::line_in`), §4.9 R3/R4 (the v2 desk sends a
//! per-1 rate, accepted when it equals the current line's `per_one`), §8 migration 4 (a per-1
//! rate is published × `currencies.unit_size`), §8 migration 6 (`rates` dropped).
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips (the callers' harnesses check it).
//!
//! The rate engine is driven by `rates_re4c_common/publish.rs` (`include!`d here, every path in
//! full); the caller still declares `mod rates_re4_common;` beside `mod rates_re4e_publish;`.
#![allow(dead_code)]

mod engine {
    #![allow(dead_code)]
    include!("../rates_re4c_common/publish.rs");
}

#[allow(unused_imports)]
pub use engine::{Published, publish_over_current, set_counter_rate};
