//! Rate feed F11 — every list of published prices shows a tiered currency at its first tier
//! (`docs/design/rate-feed-connector.md` §9.1, §9.4; owner's ruling R-10 on #986:
//! *"แสดงราคาขั้นแรก (แนะนำ)"*).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! What this pins:
//! - `GET /api/v3/views/published_rates` lists one line per `(currency, group)`: on a tiered
//!   publication, the first tier (`tier_lower_exclusive_thb IS NULL`); an untiered line as before.
//! - A source guard: every SQL string under `api/src` and `api/crates/*/src` that reads
//!   `published_rate_lines` names `tier_lower_exclusive_thb` or `value_tier` near it, except an
//!   allow-list of readers that work per line or pick the tier themselves. A new list reader
//!   cannot forget R-10.

#[macro_use]
mod auth_http_common;
mod common;

use std::path::{Path, PathBuf};

use actix_web::http::Method;
use auth_http_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const MAIN: &str = "MAIN";

/// Readers that may read `published_rate_lines` without a tier predicate (design §9.4 "Guard").
const ALLOWED: &[&str] = &[
    // picks the tier itself (line_for), reads by id, or reads the 'ALL' tier on purpose
    "crates/xc-rate/src/infrastructure/publication/published_rates.rs",
    // writes publications
    "crates/xc-rate/src/application/publication/publish_service.rs",
    "crates/xc-rate/src/application/publication/feed_publisher.rs",
    // asks whether a group is priced at all, any tier
    "crates/xc-rate/src/application/currency/denomination_group_service.rs",
    // a doc comment naming the table
    "crates/xc-kiosk/src/domain/kiosk/value_objects/counter_facts.rs",
    // reads one line by id (controller claude/553782, 2026-10-14, readers added to main after d365b04)
    "crates/xc-alert/src/infrastructure/repository.rs",
    "src/application/workflows/exchange/quote.rs",
    // DISTINCT currency codes of a publication: any tier gives the same set
    "src/presentation/http/v3/views/admin_cash.rs",
];

/// The kiosk's list readers, which F7 (#1142) moves onto the first tier. F7 removes each entry
/// as it fixes the reader.
const PENDING_F7: &[&str] = &[
    "src/infrastructure/kiosk/kiosk_ports.rs",
    "crates/xc-kiosk/src/infrastructure/kiosk/kiosk_repository.rs",
    "src/presentation/http/v3/views/kiosk_currencies.rs",
    // kiosk rate freshness (added to main after d365b04; held by BK-4 #1382's file lock)
    "src/presentation/http/kiosk.rs",
];

/// One publication on MAIN, made current, with an untiered JPY line and three USD tiers.
/// Answers the USD tier line ids, T1 first.
async fn publish_tiered(pool: &PgPool) -> Vec<String> {
    let request: String = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, 'f11-seed', 'h', 'p', '{}'::jsonb, 'test', now(), \
         'APPROVED', gen_random_uuid()::text, '00000000-0000-0000-0000-000000000001') RETURNING id::text",
    )
    .bind(MAIN)
    .fetch_one(pool)
    .await
    .expect("an approved request");
    let publication: String = sqlx::query_scalar(
        "INSERT INTO rate_publications (id, branch_id, version, approved_request_id, published_by) \
         VALUES (gen_random_uuid(), $1, \
                 (SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $1), \
                 $2::uuid, 'test') RETURNING id::text",
    )
    .bind(MAIN)
    .bind(&request)
    .fetch_one(pool)
    .await
    .expect("a publication");
    let snapshot = json!({ "buy_mode": "MANUAL", "sell_mode": "MANUAL" });
    sqlx::query(
        "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
         buy_rate, sell_rate, source_snapshot) VALUES (gen_random_uuid(), $1::uuid, 'JPY', 'ALL', 100, 23.1, 23.5, $2)",
    )
    .bind(&publication)
    .bind(&snapshot)
    .execute(pool)
    .await
    .expect("the JPY line");
    let mut ids = Vec::new();
    for (tier, lo, hi, buy, sell) in [
        ("T1", None, Some("10000.00"), "31.50", "33.00"),
        ("T2", Some("10000.00"), Some("50000.00"), "31.70", "32.80"),
        ("T3", Some("50000.00"), None, "31.90", "32.60"),
    ] {
        let id: String = sqlx::query_scalar(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
             buy_rate, sell_rate, source_snapshot, value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
             base_buy_rate, base_sell_rate) \
             VALUES (gen_random_uuid(), $1::uuid, 'USD', 'ALL', 1, $2::numeric, $3::numeric, $4, $5, \
                     $6::numeric, $7::numeric, 32.00, 32.50) RETURNING id::text",
        )
        .bind(&publication)
        .bind(buy)
        .bind(sell)
        .bind(&snapshot)
        .bind(tier)
        .bind(lo)
        .bind(hi)
        .fetch_one(pool)
        .await
        .expect("a USD tier line");
        ids.push(id);
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
        .bind(&publication)
        .bind(MAIN)
        .execute(pool)
        .await
        .expect("make it current");
    ids
}

async fn screen(app: &TestApp, owner: &str) -> Value {
    let answer = send(
        app,
        Call {
            method: Some(Method::GET),
            path: "/api/v3/views/published_rates".to_owned(),
            proto: Some("https".to_owned()),
            ..Call::default()
        }
        .session(owner),
    )
    .await;
    assert_eq!(answer.status, 200, "published_rates: {}", answer.text);
    answer.json()["data"].clone()
}

#[actix_web::test]
async fn the_rates_screen_lists_a_tiered_currency_once_at_its_first_tier() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("boss").await;
    let usd = publish_tiered(&desk.pool).await;

    let data = screen(&app, &owner).await;
    let lines = data["lines"].as_array().expect("lines");
    let usd_lines: Vec<&Value> = lines.iter().filter(|l| l["currency"] == "USD").collect();
    assert_eq!(
        usd_lines.len(),
        1,
        "one USD row, not one per tier: {lines:?}"
    );
    assert_eq!(usd_lines[0]["lineId"], json!(usd[0]), "the first tier (T1)");
    assert_eq!(usd_lines[0]["buyRate"], json!("31.50000000"));
    assert_eq!(usd_lines[0]["sellRate"], json!("33.00000000"));

    let jpy: Vec<&Value> = lines.iter().filter(|l| l["currency"] == "JPY").collect();
    assert_eq!(jpy.len(), 1, "an untiered line is listed as before");
    assert_eq!(jpy[0]["buyRate"], json!("23.10000000"));
}

fn api_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `(file relative to api/, line number)` of every read of `published_rate_lines` with no tier
/// predicate within 8 lines either side, outside the allow-lists.
fn untiered_readers(allowed: &[&str]) -> Vec<(String, usize)> {
    let root = api_dir();
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let Ok(crates) = std::fs::read_dir(root.join("crates")) else {
        panic!("api/crates exists");
    };
    for c in crates.flatten() {
        rust_files(&c.path().join("src"), &mut files);
    }
    let mut found = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&root)
            .expect("under api/")
            .to_string_lossy()
            .replace('\\', "/");
        if allowed.contains(&rel.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("readable source");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || !line.contains("published_rate_lines") {
                continue;
            }
            if line.contains("INSERT INTO published_rate_lines") {
                continue;
            }
            let from = i.saturating_sub(8);
            let to = (i + 9).min(lines.len());
            let window = lines[from..to].join("\n");
            if !window.contains("tier_lower_exclusive_thb") && !window.contains("value_tier") {
                found.push((rel.clone(), i + 1));
            }
        }
    }
    found.sort();
    found
}

#[test]
fn every_list_reader_picks_a_tier() {
    let allowed: Vec<&str> = ALLOWED.iter().chain(PENDING_F7.iter()).copied().collect();
    let found = untiered_readers(&allowed);
    assert!(
        found.is_empty(),
        "these read published_rate_lines with no tier predicate (R-10: add \
         `tier_lower_exclusive_thb IS NULL`, design §9.1): {found:?}"
    );
}

#[test]
fn the_guard_sees_an_untiered_reader() {
    // Without the allow-lists the guard must find the kiosk readers F7 has not fixed yet, and the
    // allowed per-line readers: proof that the scan reaches both src/ and crates/*/src.
    let found = untiered_readers(&[]);
    let files: Vec<&str> = found.iter().map(|(f, _)| f.as_str()).collect();
    assert!(
        files.contains(&"crates/xc-rate/src/application/currency/denomination_group_service.rs"),
        "the scan reaches crates/*/src: {found:?}"
    );
}
