//! Ticket #603 (RE-6): the kiosk's `configure` switches a currency on or off against the branch's
//! current rate publication, and never writes a rate.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §1 (the direct save is retired), §4.1 (a side that is
//! NULL is unavailable), §4.8 (every reader reads the current publication), §9 RE-6.
//!
//! What is pinned:
//! 1. `KioskService::configure` stores only the switch and its audit line. It never asks the rate
//!    context to save (`KioskPorts::save_rate` is not called), and the `rates`, `approval_requests`,
//!    `rate_publications` and `published_rate_lines` tables and `branches.current_publication_id`
//!    are unchanged by any configure. `ConfigureCurrency`'s `buy`, `sell`, `partner_*`,
//!    `previous_updated` and `request_id` are ignored: these tests send rates that v2 would have
//!    refused and rates that it would have accepted, and neither decides anything.
//! 2. `enabled = true` is accepted only when the branch's **current** publication has a line for
//!    the currency with both `buy_rate` and `sell_rate` non-NULL. Otherwise the refusal is
//!    `KioskServiceError::RatesFirst` (v2's sentence) and nothing is written: no switch change, no
//!    audit line. `enabled = false` is always accepted. THB and an unknown code stay refused with
//!    `InvalidCurrencySettings`.
//!
//! 3. The audit line (the ticket author's ruling on #603, 2026-10-09): configure sets no rate, so
//!    its `KIOSK_CURRENCY_CONFIGURED` line claims none. It records `enabled`, with `buy` and `sell`
//!    both `""`, whatever the command carried. The `KioskAudit` type is unchanged.
//!
//! The publication is seeded straight into the tables (`kiosk_publication_common`), the way RE-1's
//! publish leaves it, and `KioskService` reads it from the same database. The check goes through
//! xc-kiosk's own repository (ruling on #603): `KioskRepository::published_both_sides_in(tx,
//! branch, code) -> AppResult<bool>` in `domain/kiosk/repositories/kiosk_repository.rs`, read
//! inside the transaction that stores the switch, implemented by `PgKioskRepository`. It is not a
//! `KioskPorts` method (that file belongs to RE-4) and not SQL in `currencies.rs`. `FakePorts`
//! knows nothing about publications.
//!
//! Needs `TEST_DATABASE_URL` (see `kiosk_service_common/mod.rs`); nothing here skips.
//!
//! Signatures/fields this test assumes: `KioskService::configure(&BranchId, &KioskActor,
//! ConfigureCurrency, DateTime<Utc>) -> AppResult<()>` and `ConfigureCurrency` as they are on
//! `f04fd32`; `KioskServiceError::{RatesFirst, InvalidCurrencySettings}`; the tables of #598;
//! the new `KioskRepository::published_both_sides_in` above (reached only through `configure`).

mod kiosk_publication_common;
mod kiosk_service_common;

use std::sync::Arc;

use kiosk_publication_common::{Line, both, buy_only, publish, sell_only};
use kiosk_service_common::*;
use serde_json::{Value, json};

/// Everything a configure must not change, plus the switches and the audit log.
#[derive(Debug, PartialEq)]
struct Ledger {
    rates: i64,
    approval_requests: i64,
    publications: i64,
    lines: i64,
    current_main: Option<String>,
    current_b2: Option<String>,
    switches: Value,
    audit: Value,
}

async fn ledger(db: &TestDb) -> Ledger {
    Ledger {
        rates: db.count("rates").await,
        approval_requests: db.count("approval_requests").await,
        publications: db.count("rate_publications").await,
        lines: db.count("published_rate_lines").await,
        current_main: kiosk_publication_common::current(&db.pool, MAIN).await,
        current_b2: kiosk_publication_common::current(&db.pool, B2).await,
        switches: db.switches().await,
        audit: db.audit_since(0).await.0,
    }
}

/// The rate tables part of a ledger: what no configure may touch.
fn rate_side(l: &Ledger) -> (i64, i64, i64, i64, Option<String>, Option<String>) {
    (
        l.rates,
        l.approval_requests,
        l.publications,
        l.lines,
        l.current_main.clone(),
        l.current_b2.clone(),
    )
}

/// A configure as a screen might still send it: rates the service must ignore.
fn configure(code: &str, enabled: bool, buy: &str, sell: &str) -> ConfigureCurrency {
    ConfigureCurrency {
        code: code.to_owned(),
        enabled,
        buy: buy.to_owned(),
        sell: sell.to_owned(),
        previous_updated: "2026-10-06T04:00:00.000Z".to_owned(),
        partner_name: "Partner".to_owned(),
        partner_cost: Some("36.1".to_owned()),
        request_id: unique("req"),
    }
}

/// A configure as the RE-6 screen sends it: the code and the switch only.
fn switch_only(code: &str, enabled: bool) -> ConfigureCurrency {
    configure(code, enabled, "", "")
}

/// The stored switch of `code` in `branch`, `None` when no row was ever stored.
fn switch_of(switches: &Value, branch: &str, code: &str) -> Option<bool> {
    switches
        .as_array()
        .expect("switches is an array")
        .iter()
        .find(|s| s["branch"] == json!(branch) && s["code"] == json!(code))
        .map(|s| s["enabled"].as_bool().expect("enabled is a bool"))
}

/// The audit lines with the keys this file pins: all of them, `buy` and `sell` included.
fn pinned(audit: &Value) -> Value {
    Value::Array(
        audit
            .as_array()
            .expect("audit is an array")
            .iter()
            .map(|l| {
                json!({
                    "branch": l["branch"], "actor": l["actor"], "kind": l["kind"],
                    "subject": l["subject"], "enabled": l["enabled"],
                    "buy": l["buy"], "sell": l["sell"],
                })
            })
            .collect(),
    )
}

/// A configure's audit line: `buy` and `sell` are always `""` (item 3).
fn configured(branch: &str, actor: &str, code: &str, enabled: bool) -> Value {
    json!({
        "branch": branch, "actor": actor, "kind": "KIOSK_CURRENCY_CONFIGURED",
        "subject": code, "enabled": enabled, "buy": "", "sell": "",
    })
}

async fn start() -> (TestDb, Arc<FakePorts>, KioskService) {
    let db = TestDb::start().await;
    let ports = FakePorts::standard();
    let service = db.service(ports.clone());
    (db, ports, service)
}

// ── 1. configure never writes a rate ────────────────────────────────────────

/// Item 1 and 2: with CHF published on both sides, enabling CHF stores the switch and exactly one
/// audit line, and writes no rate anywhere, whatever rates the command carries (here ones v2 would
/// have refused: buy above sell, and text that is not a number).
#[tokio::test]
async fn enabling_a_published_currency_stores_the_switch_and_one_audit_line_and_no_rate() {
    let (db, ports, service) = start().await;
    publish(
        &db.pool,
        MAIN,
        exact(0),
        &[
            both("USD", "32.10000000", "32.45000000"),
            both("CHF", "36.40000000", "37.10000000"),
        ],
    )
    .await;
    let before = ledger(&db).await;
    assert_eq!(
        switch_of(&before.switches, MAIN, "CHF"),
        None,
        "CHF has no stored switch yet"
    );

    for (i, (buy, sell)) in [("37.2", "37.1"), ("abc", "")].into_iter().enumerate() {
        let now = at(1_000 * (i as i64 + 1));
        within(
            MUST_FINISH,
            "configure",
            service.configure(
                &branch(MAIN),
                &rates1(),
                configure("CHF", true, buy, sell),
                now,
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("enable CHF with buy {buy:?} sell {sell:?}: {e:?}"));
    }

    let after = ledger(&db).await;
    assert_eq!(
        rate_side(&after),
        rate_side(&before),
        "configure writes no rate, request or publication"
    );
    assert!(
        ports.rate_saves().is_empty(),
        "configure never asks the rate context to save: {:?}",
        ports.rate_saves()
    );
    assert_eq!(switch_of(&after.switches, MAIN, "CHF"), Some(true));
    assert_eq!(
        pinned(&after.audit),
        json!([
            configured(MAIN, "rates1", "CHF", true),
            configured(MAIN, "rates1", "CHF", true),
        ]),
        "one audit line per configure"
    );
    db.finish().await;
}

/// Item 1: switching off writes no rate either, with or without rates in the command.
#[tokio::test]
async fn disabling_writes_no_rate_and_asks_for_no_save() {
    let (db, ports, service) = start().await;
    publish(&db.pool, MAIN, exact(0), &[both("USD", "32.1", "32.45")]).await;
    let before = ledger(&db).await;

    service
        .configure(
            &branch(MAIN),
            &owner(),
            configure("USD", false, "32.1", "32.6"),
            at(1_000),
        )
        .await
        .expect("disable USD with rates in the command");
    service
        .configure(
            &branch(MAIN),
            &owner(),
            switch_only("EUR", false),
            at(2_000),
        )
        .await
        .expect("disable EUR with none");

    let after = ledger(&db).await;
    assert_eq!(rate_side(&after), rate_side(&before));
    assert!(ports.rate_saves().is_empty(), "{:?}", ports.rate_saves());
    assert_eq!(switch_of(&after.switches, MAIN, "USD"), Some(false));
    assert_eq!(switch_of(&after.switches, MAIN, "EUR"), Some(false));
    assert_eq!(
        pinned(&after.audit),
        json!([
            configured(MAIN, "owner", "USD", false),
            configured(MAIN, "owner", "EUR", false),
        ])
    );
    db.finish().await;
}

/// Item 1: a failing rate context no longer matters, because it is never asked. v2 refused this
/// configure with the rate context's sentence (the fixture's step 14).
#[tokio::test]
async fn a_rate_context_that_would_refuse_is_never_reached() {
    let (db, ports, service) = start().await;
    publish(&db.pool, MAIN, exact(0), &[both("CHF", "36.4", "37.1")]).await;
    ports.rate_save_fails(Some("เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน"));
    service
        .configure(
            &branch(MAIN),
            &rates1(),
            configure("CHF", true, "36.4", "37.1"),
            at(1_000),
        )
        .await
        .expect("the switch is stored; no rate save is asked for");
    assert!(ports.rate_saves().is_empty(), "{:?}", ports.rate_saves());
    assert_eq!(switch_of(&db.switches().await, MAIN, "CHF"), Some(true));
    db.finish().await;
}

/// Item 3: with plain, valid rates in the command (which v2 recorded in the line), the audit
/// line of an enable and of a disable still carries `buy` and `sell` as `""`.
#[tokio::test]
async fn the_audit_line_claims_no_rate() {
    let (db, _ports, service) = start().await;
    publish(&db.pool, MAIN, exact(0), &[both("CHF", "36.4", "37.1")]).await;
    service
        .configure(
            &branch(MAIN),
            &rates1(),
            configure("CHF", true, "36.4", "37.1"),
            at(1_000),
        )
        .await
        .expect("enable CHF");
    service
        .configure(
            &branch(MAIN),
            &owner(),
            configure("CHF", false, "36.5", "37.2"),
            at(2_000),
        )
        .await
        .expect("disable CHF");
    let (audit, _, _) = db.audit_since(0).await;
    assert_eq!(
        audit,
        json!([
            configured(MAIN, "rates1", "CHF", true),
            configured(MAIN, "owner", "CHF", false),
        ]),
        "every key of the line, buy and sell empty"
    );
    db.finish().await;
}

// ── 2. enabling needs a published rate ──────────────────────────────────────

/// Asks to enable `code` in `branch` and expects `RatesFirst` with nothing written.
async fn refused_rates_first(
    db: &TestDb,
    ports: &FakePorts,
    service: &KioskService,
    b: &str,
    code: &str,
    what: &str,
) {
    let before = ledger(db).await;
    let sentence = refusal(
        service
            .configure(
                &branch(b),
                &owner(),
                configure(code, true, "36.4", "37.1"),
                at(50_000),
            )
            .await,
    );
    assert_eq!(
        sentence,
        KioskServiceError::RatesFirst.to_string(),
        "{what}: v2's sentence"
    );
    assert_eq!(
        sentence, "Set valid counter buy and sell rates first.",
        "{what}: the sentence itself"
    );
    assert_eq!(ledger(db).await, before, "{what}: nothing is written");
    assert!(
        ports.rate_saves().is_empty(),
        "{what}: {:?}",
        ports.rate_saves()
    );
}

/// Item 2: before the branch's first publication, enabling is refused even with valid rates in
/// the command (v2 accepted these), and even with an old counter rate in `rates`.
#[tokio::test]
async fn enabling_before_any_publication_is_refused_rates_first_and_writes_nothing() {
    let (db, ports, service) = start().await;
    sqlx::query(
        "INSERT INTO rates (branch_id, code, buy, sell, partner_name, partner_cost, updated_at, \
         updated_by, request_id) VALUES ('MAIN', 'CHF', 36.4, 37.1, '', NULL, $1, 'test', 'r-1')",
    )
    .bind(exact(0))
    .execute(&db.pool)
    .await
    .expect("an old counter rate");
    refused_rates_first(&db, &ports, &service, MAIN, "CHF", "no publication").await;
    db.finish().await;
}

/// Item 2: a publication that has no line for the currency does not let it be enabled.
#[tokio::test]
async fn enabling_a_currency_the_publication_has_no_line_for_is_refused() {
    let (db, ports, service) = start().await;
    publish(&db.pool, MAIN, exact(0), &[both("USD", "32.1", "32.45")]).await;
    refused_rates_first(&db, &ports, &service, MAIN, "CHF", "no CHF line").await;
    db.finish().await;
}

/// Item 2: a line with either side unavailable (NULL) does not let the currency be enabled; the
/// same currency with both sides in a later publication does.
#[tokio::test]
async fn a_line_with_one_side_unavailable_is_refused_and_both_sides_are_accepted() {
    let (db, ports, service) = start().await;
    publish(&db.pool, MAIN, exact(0), &[sell_only("CHF", "37.1")]).await;
    refused_rates_first(&db, &ports, &service, MAIN, "CHF", "buy is NULL").await;

    publish(&db.pool, MAIN, exact(1_000), &[buy_only("CHF", "36.4")]).await;
    refused_rates_first(&db, &ports, &service, MAIN, "CHF", "sell is NULL").await;

    publish(&db.pool, MAIN, exact(2_000), &[both("CHF", "36.4", "37.1")]).await;
    service
        .configure(&branch(MAIN), &owner(), switch_only("CHF", true), at(3_000))
        .await
        .expect("both sides published");
    assert_eq!(switch_of(&db.switches().await, MAIN, "CHF"), Some(true));
    db.finish().await;
}

/// Item 2: only the branch's **current** publication counts. CHF in an older publication of the
/// branch does not let it be enabled once a newer one without CHF is current.
#[tokio::test]
async fn only_the_current_publication_counts() {
    let (db, ports, service) = start().await;
    publish(&db.pool, MAIN, exact(0), &[both("CHF", "36.4", "37.1")]).await;
    publish(
        &db.pool,
        MAIN,
        exact(1_000),
        &[both("USD", "32.1", "32.45")],
    )
    .await;
    refused_rates_first(&db, &ports, &service, MAIN, "CHF", "CHF only in version 1").await;
    db.finish().await;
}

/// Item 2: another branch's publication does not count; each branch is judged by its own.
#[tokio::test]
async fn another_branchs_publication_does_not_count() {
    let (db, ports, service) = start().await;
    publish(&db.pool, B2, exact(0), &[both("SGD", "24.05", "24.6")]).await;
    refused_rates_first(
        &db,
        &ports,
        &service,
        MAIN,
        "SGD",
        "SGD published in B2 only",
    )
    .await;

    service
        .configure(&branch(B2), &owner(), switch_only("SGD", true), at(1_000))
        .await
        .expect("B2 has SGD");
    let switches = db.switches().await;
    assert_eq!(switch_of(&switches, B2, "SGD"), Some(true));
    assert_eq!(switch_of(&switches, MAIN, "SGD"), None, "MAIN untouched");
    db.finish().await;
}

/// Item 2: a JPY line per 100 counts like any other (the unit plays no part in the check).
#[tokio::test]
async fn a_line_per_unit_size_counts() {
    let (db, _ports, service) = start().await;
    publish(
        &db.pool,
        MAIN,
        exact(0),
        &[Line {
            code: "KRW".to_owned(),
            unit_size: Some("1000".to_owned()),
            buy: Some("21.58500000".to_owned()),
            sell: Some("25.64500000".to_owned()),
        }],
    )
    .await;
    service
        .configure(&branch(MAIN), &owner(), switch_only("KRW", true), at(1_000))
        .await
        .expect("KRW per 1000, both sides");
    assert_eq!(switch_of(&db.switches().await, MAIN, "KRW"), Some(true));
    db.finish().await;
}

/// Item 2: switching off needs nothing published, for a currency with no line at all.
#[tokio::test]
async fn disabling_is_accepted_with_nothing_published() {
    let (db, ports, service) = start().await;
    service
        .configure(
            &branch(MAIN),
            &rates1(),
            switch_only("JPY", false),
            at(1_000),
        )
        .await
        .expect("disable JPY with no publication");
    let after = ledger(&db).await;
    assert_eq!(switch_of(&after.switches, MAIN, "JPY"), Some(false));
    assert_eq!(
        pinned(&after.audit),
        json!([configured(MAIN, "rates1", "JPY", false)])
    );
    assert!(ports.rate_saves().is_empty());
    db.finish().await;
}

/// Item 2: THB and a code that is no currency stay `InvalidCurrencySettings`, on or off, even
/// with a publication present; nothing is written.
#[tokio::test]
async fn thb_and_an_unknown_code_stay_invalid_currency_settings() {
    let (db, ports, service) = start().await;
    publish(&db.pool, MAIN, exact(0), &[both("USD", "32.1", "32.45")]).await;
    let before = ledger(&db).await;
    for (code, enabled) in [("THB", true), ("THB", false), ("XXX", true), ("usd", true)] {
        let sentence = refusal(
            service
                .configure(
                    &branch(MAIN),
                    &owner(),
                    switch_only(code, enabled),
                    at(1_000),
                )
                .await,
        );
        assert_eq!(
            sentence,
            KioskServiceError::InvalidCurrencySettings.to_string(),
            "{code} {enabled}"
        );
    }
    assert_eq!(ledger(&db).await, before, "nothing is written");
    assert!(ports.rate_saves().is_empty());
    db.finish().await;
}
