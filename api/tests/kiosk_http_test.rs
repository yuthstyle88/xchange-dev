//! Written before the implementation (ticket T12c); re-recorded from the 10-06 bundle and
//! extended for row K-HTTP (`docs/design/xchange-v3.md` §5.5.3). Do not edit, delete or weaken.
//!
//! `POST /api/kiosk` — the one route of the customer kiosk, the desk's kiosk
//! queue and the kiosk-currency page — against what v2 answered.
//!
//! The four recorded files (`tests/fixtures/kiosk/{transport,currencies,queue,queue_work}.json`,
//! written by `api/tools/v2-fixtures/gen_kiosk_http.mjs` from the 10-06 bundle on
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
//!
//! Edited by the author of ticket POL (design `docs/design/xchange-v3.md` §5.5.7): the six
//! `list` steps of `queue.json` that the 10-06 bundle answers differently from the recording
//! are held to the decided answer (`changed_since_the_recording`).
//!
//! Amended by the ticket author of #603 (RE-6, claude/e5ab6c): `configure` writes no rate
//! (item 1) and switches a currency on only when it is published on both sides (item 2). The
//! replay applies v2's rate save behind the routes (`kiosk_http_common`, `configure_behind`), and
//! three steps of `currencies.json` are held to v3's answer (`decided_by_603`). The test of a
//! failing audit line publishes the won first and no longer expects a rate to have been saved.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::Duration;
use compat_common::{Fixture, fixtures_dir};
use kiosk_http_common::{
    Counter, INVALID_REQUEST, KIOSK, KioskReplay, SERVER_ERROR, TOO_MANY_QUOTES, person,
};
use rpc_common::Rule;
use serde_json::{Value, json};

/// Amended for QU-6 #1354 (ruling issuecomment-6117792804), setup only: `accept` starts the
/// request's queue ticket as the accepting teller (queue-tickets.md §4.5), and the queue serves
/// only a teller with an ACTIVE counter, so the accepting staff of `MAIN` get one (and the `users`
/// row it references), seeded as `kiosk_accept_queue_ticket_test.rs` seeds them.
async fn open_counters(pool: &sqlx::PgPool, tellers: &[&str]) {
    for teller in tellers {
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             SELECT $2::uuid, organization_id, 'Teller', 'ACTIVE', 'test' FROM branches WHERE id = $1 \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(rpc_common::MAIN)
        .bind(teller)
        .execute(pool)
        .await
        .expect("the teller's user row");
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             SELECT organization_id, id, $2::uuid, 'ACTIVE' FROM branches WHERE id = $1 \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(rpc_common::MAIN)
        .bind(teller)
        .execute(pool)
        .await
        .expect("open the teller's counter");
    }
}

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

/// Steps of the recordings (made from the 10-05 bundle) that the 10-06 bundle answers
/// otherwise, decided by ticket POL and Q1: `list` is let through for anybody signed in
/// (`server.mjs:8026` of the 10-06 bundle; `Policy`, path `Kiosk`), so the six roles outside
/// the desk (`queue.json` #67–#72: RT1, MO1, FI1, SY1, DO1, RV1) are shown the queue MG1 was
/// shown the step before (#66); `accept` by the current holder answers `{"saved":true}`
/// (`queue.json` #105; ticket #328, row Q1). Row K-HTTP re-records these files from the 10-06 bundle.
/// #603 (RE-6): the `configure` steps of `currencies.json` v3 answers otherwise, because
/// `configure` no longer saves a rate and enables only a published currency:
/// - #27 (v2: the stale `previousUpdated` refused by the rate save): the dollar is published
///   (#24), so the switch is stored and `{"saved":true}` answered, as #24 was, with its audit line;
/// - #29 (v2: the partner rule) and #40 (v2: the Bank of Thailand down): the pound was never
///   published, so the rates-first sentence.
///
/// Answers the rules and the steps v3 accepts.
fn decided_by_603(name: &str) -> (Vec<(usize, Rule)>, Vec<usize>) {
    if name != "kiosk/currencies.json" {
        return (Vec::new(), Vec::new());
    }
    let rates_first = Rule::Differs {
        status: 400,
        sentence: "Set valid counter buy and sell rates first.",
    };
    (
        vec![
            (27, Rule::AnswersAs(24)),
            (29, rates_first.clone()),
            (40, rates_first),
        ],
        vec![27],
    )
}

fn changed_since_the_recording(name: &str) -> Vec<(usize, Rule)> {
    // The 10-06 recording answers every step the way v3 decided (`list` for every signed-in
    // role, the holder's repeat `accept`), so each step is replayed as recorded.
    // RBAC §11.2 (#3659): the owner holds `QueueOperate` again (via `EXECUTIVE` (`Customers Full`)),
    // so the owner's `list` matches v2; `RATES` (`RATE_MANAGER`) is widened with `QueueOperate`, so
    // its `list` offers waiting requests (`canAccept: true`).
    let fixture = Fixture::load(name);
    let mut rules: Vec<(usize, Rule)> = fixture
        .steps()
        .iter()
        .filter(|s| {
            s.path() == Some("/api/kiosk")
                && s.action.as_deref() == Some("list")
                && s.status == Some(200)
                && fixture.sender(s).is_some_and(|p| p.role == "RATES")
        })
        .map(|s| (s.n, Rule::RecordedWith(offered_the_waiting_queue_work)))
        .collect();
    // QU-6 #1354 (design `queue-tickets.md` §7; QU-3): `accept` starts the request's queue ticket, and a
    // teller holds one CALLED or SERVING ticket. #197–#203 are second claims by tellers who already hold
    // one (FR2 #197/#200/#203, BE1 #198/#201, MG1 #199/#202): held to `alreadyServing`, v2's claim applied
    // behind the routes, so #204, #205 and #212 keep the twelve claims v2 recorded.
    if name == "kiosk/transport.json" {
        // RBAC §11.2 (#3659, C6f): `RATES` (`RATE_MANAGER`) holds `QueueOperate`, so an empty body
        // (`{}`) on `/api/kiosk` as `RT1` (#26) is refused with `Unknown kiosk action.` like `FR1`
        // (#25), matching #63 and #64.
        rules.push((
            26,
            Rule::Differs {
                status: 400,
                sentence: "Unknown kiosk action.",
            },
        ));
    }
    if name == "kiosk/queue_work.json" {
        for n in SECOND_CLAIMS {
            let step = fixture.step(n);
            assert_eq!(
                (step.action.as_deref(), step.status),
                (Some("accept"), Some(200)),
                "{name}#{n} is a claim v2 accepted"
            );
            rules.push((n, Rule::QueueRefuses(ALREADY_SERVING)));
        }
    }
    rules
}

/// QU-6 #1354: `kiosk/queue_work.json`'s claims by a teller who already holds one.
const SECOND_CLAIMS: std::ops::RangeInclusive<usize> = 197..=203;
/// QU-3's refusal of a second CALLED or SERVING ticket, as the kiosk route answers it.
const ALREADY_SERVING: &str = "alreadyServing";

/// RBAC §11.2 (#3659): `RATES` (`RATE_MANAGER`) holds `QueueOperate`, so waiting rows of its `list`
/// can be accepted (`canAccept: true`).
fn offered_the_waiting_queue_work(answer: &mut Value) {
    if let Some(rows) = answer["requests"].as_array_mut() {
        for row in rows {
            if row["status"] == json!("WAITING") && row.get("canAccept").is_some() {
                row["canAccept"] = json!(true);
            }
        }
    }
}

async fn replayed(name: &str) -> (Fixture, usize) {
    let fixture = Fixture::load(name);
    let lines = {
        let mut replay = KioskReplay::start(&fixture).await;
        // Open shifts and counters only for the three counter roles that had shifts in the
        // recording (`FRONT`, `BACKEND`, `MANAGER`); `RATES` (`RT1`) has no open shift in the
        // recording, so its widened `accept` steps (`kiosk/queue.json` #78 and
        // `kiosk/queue_work.json` #64) reach `QueueDesk::claim_ticket` and are refused with
        // `noOpenShift`.
        let staff: Vec<&str> = fixture
            .people
            .iter()
            .filter(|p| {
                !p.is_owner()
                    && p.branch_id.as_deref() == Some(rpc_common::MAIN)
                    && matches!(p.role.as_str(), "FRONT" | "BACKEND" | "MANAGER")
            })
            .map(|p| p.id.as_str())
            .collect();
        open_counters(&replay.replay.desk.db.pool, &staff).await;
        for (n, rule) in changed_since_the_recording(name) {
            replay = replay.rule(n, rule);
        }
        let (rules, accepts) = decided_by_603(name);
        for (n, rule) in rules {
            replay = replay.rule(n, rule);
        }
        for n in accepts {
            replay = replay.v3_accepts(n);
        }
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
        let expected_widened: &[usize] = match name {
            "kiosk/queue.json" => &[77, 78, 79, 80],
            "kiosk/queue_work.json" => &[64],
            _ => &[],
        };
        assert_eq!(
            replay.replay.tally.widened, expected_widened,
            "{name}: steps widened by RBAC §11.2 (#3659)"
        );
        let expected_owner_no_open_shift: &[usize] = match name {
            "kiosk/queue.json" => &[138],
            "kiosk/queue_work.json" => &[72],
            _ => &[],
        };
        assert_eq!(
            replay.replay.tally.owner_no_open_shift, expected_owner_no_open_shift,
            "{name}: owner accept steps refused with noOpenShift (#3659, #3810)"
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
    assert_eq!(
        lines, 11,
        "ten configure steps v2 accepted and #27 that v3 accepts (#603), one line each"
    );
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

/// The 10-06 bundle's queue work (`queue-work.mjs`, `server.mjs:8015-8055`): `list` for
/// anyone signed in, with `filter` and `page` and v2's two refusals of them, fifty to a page
/// with `hasMore`; the claim answering its start time, the holder's repeat claim answering
/// the same start with no second line, the two Thai refusals of a claim (the holder's
/// username, or not ready); `notifications` with `serverNow`, `workingCount` and up to ten
/// `working` in the order they started; a second branch.
#[actix_web::test]
async fn queue_work_json_is_answered_as_v2_answered_it() {
    let (fixture, lines) = replayed("kiosk/queue_work.json").await;
    assert_eq!(
        lines, 68,
        "fifty-five requests confirmed, twelve claimed, one checked"
    );
    assert_eq!(fixture.steps().len(), 212);
}

/// What no recording shows (the 10-06 bundle's recording completes no request): a request
/// completed with both times prints them and the whole seconds between them, rounded down;
/// a request with no times prints `null` for all three (§5.5.3 data migration, item 1); and
/// claiming a completed request names its holder — the caller's own included, as v2 does
/// (`queue-work.mjs:12`).
#[actix_web::test]
async fn a_completed_request_shows_its_times_and_names_its_holder() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let fr2 = person("FR2", "FRONT", 0x12);
    let counter = Counter::open(&[fr1.clone(), fr2.clone()]).await;
    let done = counter
        .waiting_request("00000000-0000-4000-8000-0000000c0101")
        .await;
    let old = counter
        .waiting_request("00000000-0000-4000-8000-0000000c0102")
        .await;
    // `done`: claimed by FR1 at 05:10:00.250, completed at 05:12:31.999 — 151.749 s
    // `old`: completed before the columns existed, so no times at all
    for (id, started, completed) in [
        (
            &done,
            Some("2026-10-06T05:10:00.250Z"),
            Some("2026-10-06T05:12:31.999Z"),
        ),
        (&old, None, None),
    ] {
        sqlx::query(
            "UPDATE kiosk_requests SET status = 'COMPLETED', staff = $2, \
             submission_id = $3, document_ref = $4 WHERE id = $1",
        )
        .bind(id)
        .bind(&fr1.id)
        .bind(format!("sub-{id}"))
        .bind(format!("XC-{id}"))
        .execute(&counter.desk.db.pool)
        .await
        .expect("complete the request by hand");

        if started.is_some() || completed.is_some() {
            sqlx::query(
                "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, kiosk_request_id, counter_teller_id, issued_at, served_at, completed_at, idempotency_key) \
                 VALUES (gen_random_uuid(), (SELECT organization_id FROM branches WHERE id = $1), $1, current_date, 'WALK_IN', (SELECT COALESCE(MAX(sequence), 0) + 1 FROM queue_tickets WHERE branch_id = $1), 'A001', 'COMPLETED', $2, NULL, now(), $3::timestamptz, $4::timestamptz, $5) \
                 ON CONFLICT (branch_id, kiosk_request_id) DO UPDATE SET served_at = EXCLUDED.served_at, completed_at = EXCLUDED.completed_at, status = EXCLUDED.status",
            )
            .bind(rpc_common::MAIN)
            .bind(id)
            .bind(started)
            .bind(completed)
            .bind(format!("key-{id}"))
            .execute(&counter.desk.db.pool)
            .await
            .expect("put queue ticket for request");
        }
    }

    let list = counter
        .ask(Some("FR2"), "list", json!({ "filter": "completed" }))
        .await;
    assert_eq!(list.status, 200, "{}", list.text);
    let requests = list.json()["requests"].clone();
    let by_id = |id: &str| {
        requests
            .as_array()
            .expect("requests")
            .iter()
            .find(|r| r["id"] == json!(id))
            .cloned()
            .unwrap_or_else(|| panic!("{id} is listed: {requests}"))
    };
    let first = by_id(&done);
    assert_eq!(first["status"], json!("COMPLETED"));
    assert_eq!(first["staff"], json!("fr1"));
    assert_eq!(first["startedAt"], json!("2026-10-06T05:10:00.250Z"));
    assert_eq!(first["completedAt"], json!("2026-10-06T05:12:31.999Z"));
    assert_eq!(first["durationSeconds"], json!(151));
    assert_eq!(first["transactionNo"], json!(format!("XC-{done}")));
    assert_eq!(first["canAccept"], json!(false));
    assert_eq!(first["canContinue"], json!(false));
    let second = by_id(&old);
    for key in ["startedAt", "completedAt", "durationSeconds"] {
        assert_eq!(second[key], Value::Null, "{key} of a request without times");
    }

    for who in ["FR1", "FR2"] {
        counter
            .ask(Some(who), "accept", json!({ "id": done }))
            .await
            .assert_refused(
                400,
                "ออเดอร์นี้มี fr1 กำลังทำรายการแล้ว",
                &format!("{who} claims a completed request"),
            );
    }
    counter.finish().await;
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

/// The audit line is part of the step: when it cannot be written, the answer is
/// the server-error sentence with status 500 and the request did not move.
#[actix_web::test]
async fn a_step_whose_audit_line_fails_is_rolled_back() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(&[fr1]).await;
    open_counters(
        &counter.desk.db.pool,
        &["00000000-0000-4000-8000-000000000011"],
    )
    .await;
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

/// When the switch's audit line cannot be written the switch is not stored, and the answer is
/// the server-error sentence, not `saved`. Amended for #603: `configure` saves no rate (item 1)
/// and switches on only a published currency (item 2), so the won is published first, and no
/// rate of the won is in `rates` afterwards.
#[actix_web::test]
async fn a_configure_whose_audit_line_fails_stores_no_switch() {
    let counter = Counter::open(&[]).await;
    publish_behind(&counter, "KRW", "23.1", "25.9").await;
    let before: (i64,) =
        sqlx::query_as("SELECT count(*) FROM rate_publications WHERE branch_id = 'MAIN'")
            .fetch_one(&counter.desk.db.pool)
            .await
            .expect("read rate_publications");
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
    // re-expected for RE-7 #604 by the ticket author: `rates` is dropped (design §8 migration
    // 6); that configure saved no rate is now that it published nothing either
    let publications: (i64,) =
        sqlx::query_as("SELECT count(*) FROM rate_publications WHERE branch_id = 'MAIN'")
            .fetch_one(&counter.desk.db.pool)
            .await
            .expect("read rate_publications");
    assert_eq!(
        publications.0, before.0,
        "configure saved no rate and published nothing"
    );
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
    open_counters(&counter.desk.db.pool, &[&fr1.id]).await;
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
    // KC-S (#4176): v3 appends `cancelToken` after v2's last key (snapshot_common::V3_ADDED_KEYS)
    let (v2_part, added) = confirmed
        .text
        .split_once(",\"cancelToken\":")
        .unwrap_or_else(|| {
            panic!(
                "`cancelToken` comes after v2's last key: {}",
                confirmed.text
            )
        });
    assert!(
        added.ends_with('}') && !added[..added.len() - 1].contains(','),
        "`cancelToken` is the last key: {}",
        confirmed.text
    );
    let token_json = &added[..added.len() - 1];
    assert!(
        token_json.starts_with('"')
            && token_json.ends_with('"')
            && token_json.len() == 38
            && uuid::Uuid::parse_str(&token_json[1..token_json.len() - 1]).is_ok(),
        "`cancelToken` is a 36-char UUID string: {}",
        confirmed.text
    );
    assert_eq!(
        format!("{v2_part}}}"),
        "{\"id\":\"00000000-0000-4000-8000-0000000c0001\",\"queue\":\"K-00000000\",\
         \"orderNo\":\"ORD-00000000-0000-4000-8000-0000000C0001\",\"transactionNo\":null,\
         \"status\":\"WAITING\"}"
    );
    let notifications = counter.ask(Some("FR1"), "notifications", json!({})).await;
    // OR-7.m S-D2: one name per user (known difference; rulings on #3730)
    assert_eq!(
        notifications.text,
        format!(
            "{{\"serverNow\":\"2026-10-06T05:00:04.000Z\",\"workingCount\":0,\"working\":[],\
             \"branchId\":\"MAIN\",\"branchName\":\"XC-B\",\"count\":1,\
             \"latest\":\"{id}\",\"employee\":{{\"id\":\"{}\",\"username\":\"fr1\",\
             \"name\":\"fr1\",\"role\":\"FRONT\"}}}}",
            fr1.id
        )
    );
    let list = counter.ask(Some("FR1"), "list", json!({})).await;
    assert_eq!(
        list.text,
        format!(
            "{{\"branchId\":\"MAIN\",\"serverNow\":\"2026-10-06T05:00:04.000Z\",\"page\":0,\
             \"hasMore\":false,\"requests\":[{{\"paymentMethod\":\"Cash\",\"type\":\"BUY\",\
             \"currency\":\"USD\",\"rate\":32.1,\"foreign\":250,\"thb\":8025,\
             \"inputAmount\":250,\"branchName\":\"XC-B\",\"id\":\"{id}\",\"branch\":\"MAIN\",\
             \"status\":\"WAITING\",\"created\":{created},\"staff\":\"\",\"queue\":\"K-00000000\",\
             \"orderNo\":\"ORD-00000000-0000-4000-8000-0000000C0001\",\"transactionNo\":null,\
             \"startedAt\":null,\"completedAt\":null,\"durationSeconds\":null,\
             \"canAccept\":true,\"canContinue\":false}}]}}"
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
    // #3473: v3 appends `quoteId` after v2's last key (snapshot_common::V3_ADDED_KEYS);
    // v2's part is everything before it, unchanged.
    let (v2_part, added) = held
        .text
        .split_once(",\"quoteId\":")
        .unwrap_or_else(|| panic!("`quoteId` comes after v2's last key: {}", held.text));
    assert!(
        added.ends_with('}') && !added[..added.len() - 1].contains(','),
        "`quoteId` is the last key: {}",
        held.text
    );
    assert_eq!(
        format!("{v2_part}}}"),
        "{\"paymentMethod\":\"Cash\",\"type\":\"BUY\",\"currency\":\"USD\",\"rate\":32.1,\
         \"foreign\":100,\"thb\":3210,\"inputAmount\":100,\"branchName\":\"XC-B\",\
         \"id\":\"c0ffee12-0000-4000-8000-0000000c0002\",\"queue\":\"K-C0FFEE12\"}"
    );
    let rates = counter.ask(None, "rates", json!({})).await;
    // RF-5 (#877): v3 appends `freshness` after v2's last key (snapshot_common::V3_ADDED_KEYS);
    // v2's part is everything before it, unchanged.
    let (v2_part, added) = rates
        .text
        .split_once(",\"freshness\":{\"stale\":")
        .unwrap_or_else(|| panic!("`freshness` comes after v2's last key: {}", rates.text));
    assert!(
        added.ends_with("}}") && !added.contains("\"simulation\""),
        "`freshness` is the last key: {}",
        rates.text
    );
    assert!(
        v2_part.starts_with(
            "{\"branchId\":\"MAIN\",\"branchName\":\"XC-B\",\"open\":true,\"rates\":[{\"code\":\"USD\",\"name\":\""
        ) && format!("{v2_part}}}").ends_with(
            "\"buy\":32.1,\"sell\":32.6}],\"paymentMethods\":[\
             {\"id\":\"Cash\",\"enabled\":true,\"buy\":true,\"sell\":true,\"instruction\":\"\"},\
             {\"id\":\"Credit Card\",\"enabled\":true,\"buy\":false,\"sell\":true,\"instruction\":\"\"},\
             {\"id\":\"Alipay\",\"enabled\":true,\"buy\":false,\"sell\":true,\"instruction\":\"\"},\
             {\"id\":\"PromptPay\",\"enabled\":true,\"buy\":false,\"sell\":true,\"instruction\":\"\"},\
             {\"id\":\"Bank Transfer\",\"enabled\":true,\"buy\":true,\"sell\":true,\
             \"instruction\":\"If transfer, call agent · หากโอนเงิน กรุณาติดต่อพนักงาน\"}],\
             \"simulation\":false}"
        ),
        "{}",
        rates.text
    );
    counter.finish().await;
}

/// The holder pressing accept again is not refused, and another employee is (ticket #328),
/// with the 10-06 answers (K-HTTP #329, lane controller's option A).
/// QU-6 #1354 (ruling issuecomment-6117792804, item 2): an accept the queue refuses for its caller
/// (`QueueDesk`'s `QueueOperate` check on the branch's organisation) answers the queue's v3 code
/// `outOfScope` as a 400, not a server error, and nothing moves: the request and its ticket stay
/// WAITING. The kiosk's policy already holds `accept` to `QueueOperate` (`policy.rs` kiosk row), so
/// the refusal is reached here with the branch belonging to a different organisation from the
/// caller's old cookie (code "XC").
#[actix_web::test]
async fn an_accept_the_queue_refuses_answers_out_of_scope_and_the_request_stays_waiting() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let counter = Counter::open(std::slice::from_ref(&fr1)).await;
    open_counters(&counter.desk.db.pool, &[&fr1.id]).await;
    let id = counter
        .waiting_request("00000000-0000-4000-8000-0000000c0001")
        .await;
    let other_org = "00000000-0000-0000-0000-000000000002";
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1::uuid, 'OTHER', 'Other Org', 'Asia/Bangkok', now())",
    )
    .bind(other_org)
    .execute(&counter.desk.db.pool)
    .await
    .expect("insert other organisation");

    sqlx::query(
        "DO $$ \
         BEGIN \
           SET LOCAL session_replication_role = 'replica'; \
           UPDATE branches SET organization_id = '00000000-0000-0000-0000-000000000002'::uuid WHERE id = 'MAIN'; \
         END $$;",
    )
    .execute(&counter.desk.db.pool)
    .await
    .expect("repoint MAIN to other organisation");

    let answer = counter
        .ask(Some("FR1"), "accept", json!({ "id": id }))
        .await;
    assert_eq!(answer.status, 400, "{}", answer.text);
    assert_eq!(
        answer.json(),
        json!({ "error": "outOfScope" }),
        "{}",
        answer.text
    );

    let stored: (String, Option<String>) =
        sqlx::query_as("SELECT status, staff FROM kiosk_requests WHERE id = $1")
            .bind(&id)
            .fetch_one(&counter.desk.db.pool)
            .await
            .expect("read the request");
    assert_eq!(
        stored,
        ("WAITING".to_string(), None),
        "the request stays WAITING"
    );
    let ticket: (String, Option<String>) = sqlx::query_as(
        "SELECT status, counter_teller_id::text FROM queue_tickets WHERE kiosk_request_id = $1",
    )
    .bind(&id)
    .fetch_one(&counter.desk.db.pool)
    .await
    .expect("read the request's ticket");
    assert_eq!(
        ticket,
        ("WAITING".to_string(), None),
        "its ticket stays WAITING"
    );
    counter.finish().await;
}

#[actix_web::test]
async fn a_holder_accepting_again_gets_saved() {
    let fr1 = person("FR1", "FRONT", 0x11);
    let fr2 = person("FR2", "FRONT", 0x12);
    let counter = Counter::open(&[fr1.clone(), fr2.clone()]).await;
    open_counters(&counter.desk.db.pool, &[&fr1.id]).await;
    let id = counter
        .waiting_request("00000000-0000-4000-8000-0000000c0001")
        .await;

    let accepted = counter
        .ask(Some("FR1"), "accept", json!({ "id": id }))
        .await;
    assert_eq!(accepted.status, 200, "{}", accepted.text);
    let first = accepted.json();
    assert_eq!(first["saved"], json!(true), "{}", accepted.text);
    assert_eq!(first["claimed"], json!(true), "{}", accepted.text);
    assert!(first["startedAt"].is_string(), "{}", accepted.text);
    assert_eq!(first["completedAt"], Value::Null, "{}", accepted.text);
    assert_eq!(first["durationSeconds"], Value::Null, "{}", accepted.text);

    // The holder presses accept again: answered as saved, not refused, same start time.
    let repeat = counter
        .ask(Some("FR1"), "accept", json!({ "id": id }))
        .await;
    assert_eq!(repeat.status, 200, "{}", repeat.text);
    let again = repeat.json();
    assert_eq!(again["saved"], json!(true), "{}", repeat.text);
    assert_eq!(again["startedAt"], first["startedAt"], "{}", repeat.text);

    // Another employee pressing accept is refused, naming the holder.
    counter
        .ask(Some("FR2"), "accept", json!({ "id": id }))
        .await
        .assert_refused(400, "ออเดอร์นี้มี fr1 กำลังทำรายการแล้ว", "another employee");

    counter.finish().await;
}

/// #603: `MAIN`'s next publication holds `code` on both sides (behind the routes), beside the
/// dollar `Counter::open` published (RE-4b's publisher; rates per the currency's unit).
async fn publish_behind(counter: &Counter, code: &str, buy: &str, sell: &str) {
    kiosk_http_common::rates_re4_common::publish(
        &counter.desk.db.pool,
        "MAIN",
        &[("USD", "32.1", "32.6"), (code, buy, sell)],
    )
    .await;
}
