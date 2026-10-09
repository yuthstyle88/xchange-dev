//! Ticket #759 (RE-4e) — for the harnesses of RE-4e's test files that set their counter rate by
//! appending to `rates`: publish that rate set, so that it is still the counter rate once the
//! desk save, the kiosk and bookings read the current publication (RE-4b…d).
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` D-3 and §4.8 (the current publication is the counter
//! rate; every quoting reader reads `PublishedRates::line_in`), §4.9 R3/R4 (the v2 desk sends a
//! per-1 rate, accepted when it equals the current line's `per_one`), §8 migration 4 (a `rates`
//! row is per 1; a line is the same rate × `currencies.unit_size`).
//!
//! Publishing beside the old `rates` row changes nothing a pre-RE-4 reader reads: they all read
//! `rates`, and none reads publications.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips (the callers' harnesses check it).
//!
//! Signatures this module assumes: `rates_re4_common::{publish, Published}` (merged with RE-4a);
//! the caller declares `mod rates_re4_common;` beside `mod rates_re4e_publish;`.
#![allow(dead_code)]

use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::rates_re4_common::{Published, publish};

/// Publish `branch`'s newest `rates` row of every currency (per 1, as the direct save appends
/// it), × `currencies.unit_size` as migration 4 converts them, as the branch's next publication:
/// the rate set its counter quoted before RE-4 (JPY 0.2 per 1 is published as 20 per 100).
pub async fn publish_newest_rates(pool: &PgPool, branch: &str) -> Published {
    let newest: Vec<(String, Decimal, Decimal)> = sqlx::query_as(
        "SELECT DISTINCT ON (r.code) trim(r.code), r.buy * c.unit_size, r.sell * c.unit_size \
         FROM rates r JOIN currencies c ON c.code = r.code \
         WHERE r.branch_id = $1 ORDER BY r.code, r.id DESC",
    )
    .bind(branch)
    .fetch_all(pool)
    .await
    .expect("read the branch's newest rates");
    let texts: Vec<(String, String, String)> = newest
        .into_iter()
        .map(|(code, buy, sell)| {
            (
                code,
                buy.normalize().to_string(),
                sell.normalize().to_string(),
            )
        })
        .collect();
    let lines: Vec<(&str, &str, &str)> = texts
        .iter()
        .map(|(c, b, s)| (c.as_str(), b.as_str(), s.as_str()))
        .collect();
    publish(pool, branch, &lines).await
}
