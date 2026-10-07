//! Written before the implementation (ticket T12c). Do not edit, delete or weaken.
//!
//! `POST /api/kiosk` — the one route of the customer kiosk, the desk's kiosk
//! queue and the kiosk-currency page — against what v2 answered.
//!
//! The three recorded files (`tests/fixtures/kiosk/{transport,currencies,queue}.json`,
//! written by `api/tools/v2-fixtures/gen_kiosk_http.mjs` from v2's whole bundle on
//! PostgreSQL) are replayed step by step through the server's own route table:
//! status, `Cache-Control`, content type, the body, and the audit lines each
//! step wrote (`tests/fixtures/kiosk/behind.json`).
//!
//! Where v3 answers something else than v2, the step is listed here with the
//! reason; nothing is skipped silently. After the replays come the tests of
//! what no recording can show: the limit on quotes v2 does not have, whose
//! name the queue prints, what is stored as the holder of a request, and what
//! a failure behind the route leaves.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use chrono::Duration;
use compat_common::{Fixture, fixtures_dir};
use kiosk_http_common::{
    ACCEPT_FIRST, ALREADY_ASSIGNED, Counter, INVALID_REQUEST, KIOSK, KioskReplay, SERVER_ERROR,
    TOO_MANY_QUOTES, person,
};
use rpc_common::{Call, Rule};
use serde_json::{Value, json};

/// The steps v2 answered with a sentence of the JavaScript engine (a body that
/// is not JSON, or `null`): v3 says the route's own "Invalid request.", as
/// `/api/rpc` does for the same bodies (T11e).
fn engine_sentences(fixture: &Fixture) -> Vec<usize> {
    fixture
        .steps()
        .iter()
        .filter(|s| s.path() == Some(KIOSK))
        .filter(|s| {
            s.error().is_some_and(|e| {
                e.starts_with("Unexpected token")
                    || e.starts_with("Cannot destructure")
                    || e.starts_with("Expected ")
            })
        })
        .map(|s| s.n)
        .collect()
}

async fn replayed(name: &str) -> (Fixture, usize) {
    let fixture = Fixture::load(name);
    let lines = {
        let mut replay = KioskReplay::start(&fixture).await;
        for n in engine_sentences(&fixture) {
            replay = replay.rule(
                n,
                Rule::Differs {
                    status: 400,
                    sentence: INVALID_REQUEST,
                },
            );
        }
        replay.run().await;
        assert!(
            replay.replay.tally.excluded.is_empty() && replay.replay.tally.part_two.is_empty(),
            "{name}: every step is sent and compared"
        );
        let lines = replay.lines_compared;
        replay.finish().await;
        lines
    };
    (fixture, lines)
}

// ── the three recorded files ─────────────────────────────────────────────────

/// What the route does with a request before any action runs: the same-origin
/// rule (for the customer, an employee and the owner alike), the body, the
/// limit of two million characters, the method, the branch a request is for,
/// names it does not know, and an employee whose first password is unchanged
/// (the customer, to this route).
#[actix_web::test]
async fn transport_json_is_answered_as_v2_answered_it() {
    let (fixture, lines) = replayed("kiosk/transport.json").await;
    assert_eq!(
        engine_sentences(&fixture).len(),
        3,
        "the three engine sentences"
    );
    assert_eq!(lines, 0, "nothing in this file writes an audit line");
    assert_eq!(fixture.steps().len(), 74);
}

/// The kiosk-currency page: `manage` and `configure` by the owner and the rates
/// role, the rate saved through the rate rules under the name of who saved it
/// (the two `adminSnapshot` reads show it), the switch stored, one audit line
/// each — and what `rates` offers the customer afterwards, in two branches.
/// The role is judged before any field (the last three steps).
#[actix_web::test]
async fn currencies_json_is_answered_as_v2_answered_it() {
    let (fixture, lines) = replayed("kiosk/currencies.json").await;
    assert_eq!(lines, 10, "ten accepted configure steps, one line each");
    assert_eq!(fixture.steps().len(), 54);
}

/// A day at the kiosk: quotes and what a quote must be, the three-second and
/// five-minute rules, the queue, accepting, the passport check, the receipt
/// picture and its ten minutes, who may ask for what (all ten roles, the owner
/// and nobody), a second branch, and the counter closing.
#[actix_web::test]
async fn queue_json_is_answered_as_v2_answered_it() {
    let (fixture, lines) = replayed("kiosk/queue.json").await;
    assert_eq!(
        lines, 8,
        "three requests confirmed, three accepted, two checked"
    );
    assert_eq!(fixture.steps().len(), 171);
}

/// The table of currencies a kiosk can offer is v2's: 152 codes with their
/// Thai and English names and countries, in v2's order, the baht not among
/// them. (`manage` of an empty shop shows the whole table.)
#[actix_web::test]
async fn manage_lists_v2s_currency_table() {
    let path = fixtures_dir().join("kiosk/currency_table.json");
    let table: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("currency_table.json"))
            .expect("JSON");
    let table = table["currencies"].as_array().expect("a list").clone();
    assert_eq!(table.len(), 152);

    let counter = Counter::open(&[]).await;
    let answer = counter.ask(Some("owner"), "manage", json!({})).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let listed = answer.json()["currencies"]
        .as_array()
        .expect("a list")
        .clone();
    assert_eq!(listed.len(), table.len());
    for (row, entry) in listed.iter().zip(&table) {
        for key in ["code", "name", "nameEn", "countries"] {
            assert_eq!(row[key], entry[key], "{} {key}", entry["code"]);
        }
        assert_ne!(row["code"], "THB");
        // the keys v2 prints, in v2's order
        let keys: Vec<&str> = row
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert!(
            keys.contains(&"enabled") && keys.contains(&"botReference"),
            "{keys:?}"
        );
    }
    // v2's bytes begin with the first currency's code, then its names
    assert!(
        answer
            .text
            .starts_with("{\"currencies\":[{\"code\":\"AED\",\"name\":\""),
        "{:.80}",
        answer.text
    );
    counter.finish().await;
}

// ── what no recording shows ──────────────────────────────────────────────────

/// v3's own rule (design §6): at most 20 quotes a minute from one client
/// address, which is the `X-Real-IP` header nginx sets. The 21st is refused,
/// another address is not, and a minute later the first address may ask again.
#[actix_web::test]
async fn quotes_are_limited_per_client_address() {
    let counter = Counter::open(&[]).await;
    let quote = json!({ "type": "BUY", "currency": "USD", "amount": 1 });
    for i in 0..20 {
        let answer = counter
            .ask_from(None, "quote", quote.clone(), Some("203.0.113.7"))
            .await;
        assert_eq!(answer.status, 200, "quote {i}: {}", answer.text);
    }
    let refused = counter
        .ask_from(None, "quote", quote.clone(), Some("203.0.113.7"))
        .await;
    refused.assert_refused(400, TOO_MANY_QUOTES, "the 21st quote of one address");
    assert_eq!(refused.header("cache-control"), Some("no-store"));

    let other = counter
        .ask_from(None, "quote", quote.clone(), Some("203.0.113.8"))
        .await;
    assert_eq!(other.status, 200, "another address: {}", other.text);

    // the other actions of the same address are not counted
    let rates = counter
        .ask_from(None, "rates", json!({}), Some("203.0.113.7"))
        .await;
    assert_eq!(rates.status, 200, "{}", rates.text);

    counter
        .desk
        .clock
        .set(counter.now() + Duration::seconds(61));
    let later = counter
        .ask_from(None, "quote", quote, Some("203.0.113.7"))
        .await;
    assert_eq!(later.status, 200, "a minute later: {}", later.text);

    let stored: Vec<(String, i64)> = sqlx::query_as(
        "SELECT client_ip, count(*) FROM kiosk_requests GROUP BY client_ip ORDER BY client_ip",
    )
    .fetch_all(&counter.desk.db.pool)
    .await
    .expect("read kiosk_requests");
    assert_eq!(
        stored,
        vec![
            ("203.0.113.7".to_string(), 21),
            ("203.0.113.8".to_string(), 1)
        ],
        "the address is stored with the quote"
    );
    counter.finish().await;
}

/// The holder of a request is stored as the employee's id — a name can be
/// given to somebody else, an id cannot — and every later check compares ids.
/// Where v2 prints the holder (`staff` of `list`) v3 prints the username of
/// that id, also after the account was disabled; an audit line names the
/// username as v2's did.
#[actix_web::test]
async fn the_holder_is_stored_by_id_and_printed_by_username() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let fr2 = person("FR2", "FRONT", 0x12);
    let counter = Counter::open(&[fr1.clone(), fr2.clone()]).await;
    let id = counter
        .waiting_request("00000000-0000-4000-8000-0000000c0001")
        .await;

    let accepted = counter
        .ask(Some("FR1"), "accept", json!({ "id": id }))
        .await;
    assert_eq!(accepted.status, 200, "{}", accepted.text);
    assert_eq!(accepted.text, "{\"saved\":true}");

    let stored: (Option<String>,) =
        sqlx::query_as("SELECT staff FROM kiosk_requests WHERE id = $1")
            .bind(&id)
            .fetch_one(&counter.desk.db.pool)
            .await
            .expect("read the request");
    assert_eq!(stored.0.as_deref(), Some(fr1.id.as_str()), "stored by id");

    let (lines, _) = kiosk_http_common::audit_since(&counter.desk, 0).await;
    let accepted_line = lines
        .iter()
        .find(|line| line.kind == "KIOSK_ACCEPTED")
        .expect("an audit line for the accept");
    assert_eq!(
        accepted_line.actor, "fr1",
        "the audit line names the username"
    );
    assert_eq!(lines[0].actor, "CUSTOMER-KIOSK");

    let list = counter.ask(Some("FR2"), "list", json!({})).await;
    assert_eq!(list.json()["requests"][0]["staff"], "fr1", "{}", list.text);

    // somebody else, and the owner, do not hold it
    counter
        .ask(Some("FR2"), "get", json!({ "id": id }))
        .await
        .assert_refused(400, ACCEPT_FIRST, "another employee");
    counter
        .ask(Some("owner"), "get", json!({ "id": id }))
        .await
        .assert_refused(400, ACCEPT_FIRST, "the owner");
    counter
        .ask(Some("FR2"), "accept", json!({ "id": id }))
        .await
        .assert_refused(400, ALREADY_ASSIGNED, "accepting what is held");

    // the account is disabled: the queue still prints who holds the request
    let call = Call::action(
        &counter.origin,
        "/api/employees",
        "disable",
        json!({ "id": fr1.id }),
    );
    let disabled = counter
        .app
        .send(kiosk_http_common::as_who(call, &counter.cookies, "owner"))
        .await;
    assert_eq!(disabled.status, 200, "disable: {}", disabled.text);
    let list = counter.ask(Some("FR2"), "list", json!({})).await;
    assert_eq!(list.json()["requests"][0]["staff"], "fr1", "{}", list.text);

    // the owner holds a request under the owner's username
    let second = counter
        .waiting_request("00000000-0000-4000-8000-0000000c0002")
        .await;
    let accepted = counter
        .ask(Some("owner"), "accept", json!({ "id": second }))
        .await;
    assert_eq!(accepted.status, 200, "{}", accepted.text);
    let list = counter.ask(Some("owner"), "list", json!({})).await;
    assert_eq!(
        list.json()["requests"][0]["staff"],
        "owner",
        "{}",
        list.text
    );
    let stored: (Option<String>,) =
        sqlx::query_as("SELECT staff FROM kiosk_requests WHERE id = $1")
            .bind(&second)
            .fetch_one(&counter.desk.db.pool)
            .await
            .expect("read the request");
    assert_eq!(stored.0.as_deref(), Some("owner"), "the owner's actor id");

    // it is the owner's username that is printed, not the constant id
    sqlx::query("UPDATE owners SET username = 'boss'")
        .execute(&counter.desk.db.pool)
        .await
        .expect("rename the owner");
    let list = counter.ask(Some("FR2"), "list", json!({})).await;
    assert_eq!(list.json()["requests"][0]["staff"], "boss", "{}", list.text);

    // an id nobody has (an account removed by hand) is printed as it is
    sqlx::query("UPDATE kiosk_requests SET staff = 'gone-0001' WHERE id = $1")
        .bind(&id)
        .execute(&counter.desk.db.pool)
        .await
        .expect("orphan the first request");
    let list = counter.ask(Some("FR2"), "list", json!({})).await;
    assert_eq!(
        list.json()["requests"][1]["staff"],
        "gone-0001",
        "{}",
        list.text
    );
    counter.finish().await;
}

/// The audit line is part of the step: when it cannot be written, the answer is
/// the server-error sentence with status 500 and the request did not move.
#[actix_web::test]
async fn a_step_whose_audit_line_fails_is_rolled_back() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(&[fr1]).await;
    let id = counter
        .waiting_request("00000000-0000-4000-8000-0000000c0001")
        .await;

    counter.hands.audit.fail(true);
    let answer = counter
        .ask(Some("FR1"), "accept", json!({ "id": id }))
        .await;
    assert_eq!(answer.status, 500, "{}", answer.text);
    assert_eq!(answer.json(), json!({ "error": SERVER_ERROR }));

    let stored: (String, Option<String>) =
        sqlx::query_as("SELECT status, staff FROM kiosk_requests WHERE id = $1")
            .bind(&id)
            .fetch_one(&counter.desk.db.pool)
            .await
            .expect("read the request");
    assert_eq!(stored, ("WAITING".to_string(), None), "nothing moved");

    counter.hands.audit.fail(false);
    let answer = counter
        .ask(Some("FR1"), "accept", json!({ "id": id }))
        .await;
    assert_eq!(answer.status, 200, "once the log is back: {}", answer.text);
    counter.finish().await;
}

/// `configure` saves the rate first and stores the switch after it, as v2 does.
/// When the switch's audit line cannot be written the switch is not stored;
/// the rate, saved in a transaction of its own before it, stays — and the
/// answer is the server-error sentence, not `saved`.
#[actix_web::test]
async fn a_configure_whose_audit_line_fails_stores_no_switch() {
    let counter = Counter::open(&[]).await;
    counter.hands.audit.fail(true);
    let answer = counter
        .ask(
            Some("owner"),
            "configure",
            json!({ "code": "KRW", "enabled": true, "buy": 0.0231, "sell": 0.0259 }),
        )
        .await;
    assert_eq!(answer.status, 500, "{}", answer.text);
    assert_eq!(answer.json(), json!({ "error": SERVER_ERROR }));

    let switches: (i64,) = sqlx::query_as("SELECT count(*) FROM kiosk_currencies")
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("read kiosk_currencies");
    assert_eq!(switches.0, 0, "no switch was stored");
    counter.hands.audit.fail(false);
    let rates = counter.ask(None, "rates", json!({})).await;
    let offered: Vec<String> = rates.json()["rates"]
        .as_array()
        .expect("rates")
        .iter()
        .map(|r| r["code"].as_str().expect("a code").to_string())
        .collect();
    assert_eq!(
        offered,
        vec!["USD"],
        "the won is not offered: {}",
        rates.text
    );
    counter.finish().await;
}

/// The answers are v2's bytes, key order included: a quote, the identity of a
/// request, the queue, and the notification count.
#[actix_web::test]
async fn the_answers_keep_v2s_key_order() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    counter
        .hands
        .ids
        .push("00000000-0000-4000-8000-0000000c0001");
    let quote = counter
        .ask(
            None,
            "quote",
            json!({ "type": "BUY", "currency": "USD", "amount": 250 }),
        )
        .await;
    assert_eq!(
        quote.text,
        "{\"id\":\"00000000-0000-4000-8000-0000000c0001\",\"queue\":\"K-00000000\",\
         \"orderNo\":\"ORD-00000000-0000-4000-8000-0000000C0001\",\"transactionNo\":null,\
         \"status\":\"QUOTED\",\"paymentMethod\":\"Cash\",\"type\":\"BUY\",\"currency\":\"USD\",\
         \"rate\":32.1,\"foreign\":250,\"thb\":8025,\"inputAmount\":250,\"branchName\":\"XC-B\"}"
    );
    let created = counter.now().timestamp_millis();
    counter.desk.clock.set(counter.now() + Duration::seconds(4));
    let id = "00000000-0000-4000-8000-0000000c0001";
    let confirmed = counter.ask(None, "confirm", json!({ "id": id })).await;
    assert_eq!(
        confirmed.text,
        "{\"id\":\"00000000-0000-4000-8000-0000000c0001\",\"queue\":\"K-00000000\",\
         \"orderNo\":\"ORD-00000000-0000-4000-8000-0000000C0001\",\"transactionNo\":null,\
         \"status\":\"WAITING\"}"
    );
    let notifications = counter.ask(Some("FR1"), "notifications", json!({})).await;
    assert_eq!(
        notifications.text,
        format!(
            "{{\"branchId\":\"MAIN\",\"branchName\":\"XC-B\",\"count\":1,\
             \"latest\":\"{id}\",\"employee\":{{\"id\":\"{}\",\"username\":\"fr1\",\
             \"name\":\"FR1 of the test\",\"role\":\"FRONT\"}}}}",
            fr1.id
        )
    );
    let list = counter.ask(Some("FR1"), "list", json!({})).await;
    assert_eq!(
        list.text,
        format!(
            "{{\"requests\":[{{\"id\":\"{id}\",\"branch_id\":\"MAIN\",\"status\":\"WAITING\",\
             \"created\":{created},\"staff\":\"\",\"branch\":\"MAIN\",\"paymentMethod\":\"Cash\",\
             \"type\":\"BUY\",\"currency\":\"USD\",\"rate\":32.1,\"foreign\":250,\"thb\":8025,\
             \"inputAmount\":250,\"branchName\":\"XC-B\",\"queue\":\"K-00000000\",\
             \"orderNo\":\"ORD-00000000-0000-4000-8000-0000000C0001\",\"transactionNo\":null}}]}}"
        )
    );
    let receipt = counter
        .ask(None, "customerReceipt", json!({ "id": id }))
        .await;
    assert_eq!(
        receipt.text,
        "{\"image\":null,\"id\":\"00000000-0000-4000-8000-0000000c0001\",\"queue\":\"K-00000000\",\
         \"orderNo\":\"ORD-00000000-0000-4000-8000-0000000C0001\",\"transactionNo\":null,\
         \"status\":\"WAITING\"}"
    );
    // an id with letters: the queue number is its first eight characters in capitals
    let lettered = counter
        .waiting_request("c0ffee12-0000-4000-8000-0000000c0002")
        .await;
    let accepted = counter
        .ask(Some("FR1"), "accept", json!({ "id": lettered }))
        .await;
    assert_eq!(accepted.status, 200, "{}", accepted.text);
    let held = counter
        .ask(Some("FR1"), "get", json!({ "id": lettered }))
        .await;
    assert_eq!(
        held.text,
        "{\"paymentMethod\":\"Cash\",\"type\":\"BUY\",\"currency\":\"USD\",\"rate\":32.1,\
         \"foreign\":100,\"thb\":3210,\"inputAmount\":100,\"branchName\":\"XC-B\",\
         \"id\":\"c0ffee12-0000-4000-8000-0000000c0002\",\"queue\":\"K-C0FFEE12\"}"
    );
    let rates = counter.ask(None, "rates", json!({})).await;
    assert!(
        rates.text.starts_with(
            "{\"branchId\":\"MAIN\",\"branchName\":\"XC-B\",\"open\":true,\"rates\":[{\"code\":\"USD\",\"name\":\""
        ) && rates.text.ends_with("\"buy\":32.1,\"sell\":32.6}],\"simulation\":false}"),
        "{}",
        rates.text
    );
    counter.finish().await;
}
