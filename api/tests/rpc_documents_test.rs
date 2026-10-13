//! Written before the implementation (ticket T11a-D). Do not edit, delete or weaken.
//!
//! The `documents` action of `POST /api/staff/rpc` — the links to the papers of
//! one exchange — against what v2 recorded, **through HTTP**, with the real
//! sessions of ticket T4c.
//!
//! The recorded day is `snapshot/transactions.json`: steps #1–#23 make it (two
//! rates, a shift, customers, seven uploads and six saves — the uploads and the
//! saves applied behind the routes by `rpc_common::Replay` for as long as their
//! own tickets have not wired them), steps #51–#55 ask for documents. #24 is a
//! row the generator wrote (no request) and #25–#50 are views; neither changes
//! what `documents` reads, so they are not sent here.
//!
//! What v2 does (`xchange-online@3ffc5a5`, `server.mjs`): the staff bridge lets
//! `documents` through for every role but RATES and SYSTEM; the engine looks the
//! exchange up with `row[0] === input.ref` in the caller's own branch and
//! answers `{ref, links}` — `ref` first — or `ไม่พบเอกสารรายการ`; the bridge
//! then turns `/files/` into `/api/staff/file/`. On `/api/rpc` `documents` is
//! not an action for anybody.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use compat_common::{Fixture, Person};
use rpc_common::{
    Answer, Call, Desk, EMPLOYEE_AUTH_REQUIRED, INVALID_REQUEST, MAIN, ROLE_NOT_ALLOWED_RPC, RPC,
    Replay, SERVER_ERROR, STAFF_RPC, one_of_each_role, route_rule, sign_in, text_with_ids,
};
use serde_json::{Map, Value, json};
use snapshot_common::time;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::application::compat::actions::RpcCall;
use xchange::application::compat::envelope::RpcRoute;
use xchange::application::compat::rpc::{RpcCaller, RpcFailure};
use xchange::presentation::http::rpc::registered_actions;

const ORIGIN: &str = "http://desk.test";
const DAY: &str = "snapshot/transactions.json";

const NO_DOCUMENTS: &str = "ไม่พบเอกสารรายการ";
const DOCUMENTS_NOT_READABLE: &str = "บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้า";

/// The exchange of #10: bought for cash by the owner — a passport and a receipt.
const CASH: &str = "XC-20261006-000001";
/// The exchange of #18: a transfer saved by a Front — with a payment proof.
const TRANSFER: &str = "XC-20261006-000003";
/// The DEMO number v2 gave #23. Re-expected for TX-6 #674 by the test-writer (decision A2, owner
/// 2026-10-11, design quote-transaction.md §4.6.1): #23 is now refused, so no save made it.
const DEMO: &str = "TEST-XC-20261006-000001";

/// The last step of the recorded day that writes something.
const LAST_WRITE: usize = 23;

/// The recorded day up to its last write, every step through
/// `rpc_common::Replay` (so held to v2, or applied behind the routes, exactly
/// as `rpc_test.rs` does it).
async fn the_recorded_day(fixture: &Fixture) -> Replay<'_> {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, fixture).await;
    for step in fixture.steps().iter().filter(|s| s.n <= LAST_WRITE) {
        replay.step(step).await;
    }
    // the day as recorded: the saves answered the numbers the tests ask for
    assert_eq!(fixture.step(18).json()["documentRef"], json!(TRANSFER));
    // decision A2 (#674): #23, a DEMO SELL at 99 against the published 32.6, is refused RATE_MISMATCH
    assert_eq!(fixture.step(23).status, Some(400));
    assert_eq!(
        fixture.step(23).json()["error"],
        json!("เรทไม่ตรงกับเรทหน้าร้านล่าสุด กรุณาตรวจสอบเรทก่อนยืนยันรายการ")
    );
    replay
}

/// `documents` asked on the staff route by `who` (a key of the fixture's people).
async fn documents(replay: &Replay<'_>, who: &str, payload: Value) -> Answer {
    replay
        .app
        .send(Call::action(ORIGIN, STAFF_RPC, "documents", payload).cookie(&replay.cookies[who]))
        .await
}

/// v2's own text of an accepted step, with the keys of the uploaded files
/// replaced by the ones this run made.
fn v2_text(replay: &Replay<'_>, n: usize) -> String {
    let fixture = replay.fixture;
    let text = fixture.body_text(n).expect("an accepted step has a body");
    text_with_ids(&replay.ids, fixture.step(n).json(), &text)
}

fn actor_of(person: &Person) -> Actor {
    if person.is_owner() {
        return Actor::owner("owner");
    }
    Actor::staff(
        person.id.clone(),
        Role::parse(&person.role).expect("a role"),
        BranchId::new(person.branch_id.clone().expect("a branch")),
    )
    .expect("an employee")
}

fn front_of(branch: &str, n: u8) -> Person {
    Person {
        key: format!("FRONT-{branch}"),
        kind: "staff".to_string(),
        id: format!("00000000-0000-4000-8000-0000000000{n:02x}"),
        username: format!("front{n}"),
        name: format!("Front of {branch}"),
        role: "FRONT".to_string(),
        branch_id: Some(branch.to_string()),
        must_change: false,
    }
}

// ── snapshot/transactions.json #51–#55 ───────────────────────────────────────

/// The five `documents` steps of the recorded day, each held to what v2
/// answered — status, headers and body: a Front and a Documents employee are
/// answered (#51, #52), a Rates employee is refused in the bridge's sentence
/// (#53), a number nobody was given is `ไม่พบเอกสารรายการ` (#54), and on
/// `/api/rpc` the name is not an action even for the owner (#55).
#[actix_web::test]
async fn the_documents_steps_of_the_recorded_day_are_answered_as_v2_answered_them() {
    let fixture = Fixture::load(DAY);
    let mut replay = the_recorded_day(&fixture).await;
    let asked = [51, 52, 53, 54, 55];
    for n in asked {
        let step = fixture.step(n);
        assert_eq!(step.action.as_deref(), Some("documents"), "#{n}");
        replay.step(step).await;
    }
    // held to v2's recorded answer — not to the unknown-name refusal of the gap
    for n in asked {
        assert!(
            replay.tally.replayed.contains(&n),
            "transactions#{n} was not held to what v2 answered: {:?}",
            replay.tally
        );
        assert!(!replay.tally.part_two.contains(&n), "transactions#{n}");
    }
    assert_eq!(replay.answers[&51].status, 200);
    assert_eq!(replay.answers[&52].status, 200);
    replay.answers[&53].assert_refused(400, DOCUMENTS_NOT_READABLE, "#53");
    replay.answers[&54].assert_refused(400, NO_DOCUMENTS, "#54");
    replay.answers[&55].assert_refused(400, INVALID_REQUEST, "#55");
    replay.finish().await;
}

/// An accepted answer is v2's text byte for byte: `ref` before `links`, the
/// labels in v2's order (passport, payment proof when there is one, receipt),
/// the staff path of every file — and the receipt's key is the one v2 made.
#[actix_web::test]
async fn an_accepted_answer_is_v2s_text_byte_for_byte() {
    let fixture = Fixture::load(DAY);
    let replay = the_recorded_day(&fixture).await;

    let cash = documents(&replay, "FR1", json!({ "ref": CASH })).await;
    assert_eq!(cash.status, 200, "{:.300}", cash.text);
    assert_eq!(cash.text, v2_text(&replay, 51), "#51");
    assert!(
        cash.text.starts_with(
            "{\"ref\":\"XC-20261006-000001\",\"links\":[[\"พาสปอร์ต\",\"/api/staff/file/"
        )
    );
    assert!(
        cash.text.ends_with(
            "\"],[\"ใบเสร็จ\",\"/api/staff/file/receipt-\
             f3afb29a5351012c19c005d98e139a0e6c5aacc94bc57963342e455e6032e567\"]]}"
        ),
        "{}",
        cash.text
    );
    assert_eq!(cash.header("cache-control"), Some("no-store"));
    assert_eq!(cash.header("content-type"), Some("application/json"));

    let transfer = documents(&replay, "DO1", json!({ "ref": TRANSFER })).await;
    assert_eq!(transfer.status, 200, "{:.300}", transfer.text);
    assert_eq!(transfer.text, v2_text(&replay, 52), "#52");
    let links = transfer.json()["links"].clone();
    let labels: Vec<&str> = links
        .as_array()
        .expect("links")
        .iter()
        .map(|link| link[0].as_str().expect("a label"))
        .collect();
    assert_eq!(labels, ["พาสปอร์ต", "หลักฐานชำระเงิน", "ใบเสร็จ"]);
    replay.finish().await;
}

// ── who may ask ──────────────────────────────────────────────────────────────

/// Every employee of the recorded day on the staff route: the four roles the
/// bridge lets read are answered the same text, RATES and SYSTEM its sentence;
/// and nobody — not the owner, not a Front — is answered on `/api/rpc`.
#[actix_web::test]
async fn every_role_but_rates_and_system_reads_the_documents_of_the_branch() {
    let fixture = Fixture::load(DAY);
    let replay = the_recorded_day(&fixture).await;
    let expected = v2_text(&replay, 51);

    for who in ["FR1", "BE1", "MO1", "DO1"] {
        let answer = documents(&replay, who, json!({ "ref": CASH })).await;
        assert_eq!(answer.status, 200, "{who}: {:.300}", answer.text);
        assert_eq!(answer.text, expected, "{who}");
    }
    for who in ["RT1", "SY1"] {
        documents(&replay, who, json!({ "ref": CASH }))
            .await
            .assert_refused(400, DOCUMENTS_NOT_READABLE, who);
    }
    // the owner has no employee session
    let owner = documents(&replay, "owner", json!({ "ref": CASH })).await;
    owner.assert_refused(401, EMPLOYEE_AUTH_REQUIRED, "the owner on the staff route");

    for (who, sentence) in [("owner", INVALID_REQUEST), ("FR1", ROLE_NOT_ALLOWED_RPC)] {
        let answer = replay
            .app
            .send(
                Call::action(ORIGIN, RPC, "documents", json!({ "ref": CASH }))
                    .cookie(&replay.cookies[who]),
            )
            .await;
        answer.assert_refused(400, sentence, &format!("{who} on /api/rpc"));
    }
    replay.finish().await;
}

/// The whole role table, with an exchange nobody was given: where `Policy`
/// lets a caller through, the lookup answers (`ไม่พบเอกสารรายการ` — not the
/// role sentence of a name nobody knows); everywhere else `Policy`'s refusal.
/// Seven roles get through, all on the staff route.
#[actix_web::test]
async fn the_role_table_of_documents_is_policys_on_both_routes() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;

    let mut let_through = Vec::new();
    for person in &people {
        let actor = actor_of(person);
        for (path, entry) in [(RPC, EntryPath::Rpc), (STAFF_RPC, EntryPath::StaffRpc)] {
            let answer = app
                .send(
                    Call::action(ORIGIN, path, "documents", json!({ "ref": "XC-NOPE" }))
                        .cookie(&cookies[&person.key]),
                )
                .await;
            let label = format!("documents on {path} as {}", person.key);
            // OR-4b (#767): the route table's rule (`route_rule`); the staff route reads the staff
            // session only, so the owner's is nobody there
            let caller = (path == RPC || person.key != "OWNER").then_some(&actor);
            match route_rule(caller, entry, "documents") {
                Err(refusal) => {
                    answer.assert_refused(refusal.status(), &refusal.to_string(), &label)
                }
                Ok(()) => {
                    answer.assert_refused(400, NO_DOCUMENTS, &label);
                    assert_eq!(answer.header("cache-control"), Some("no-store"), "{label}");
                    let_through.push((person.key.clone(), path));
                }
            }
        }
    }
    assert_eq!(let_through.len(), 7, "{let_through:?}");
    assert!(let_through.iter().all(|(_, path)| *path == STAFF_RPC));
    assert!(
        !let_through
            .iter()
            .any(|(who, _)| who == "RATES" || who == "SYSTEM")
    );
    // signed in: refused for the role, not for the session
    app.send(
        Call::action(ORIGIN, STAFF_RPC, "documents", json!({ "ref": "XC-NOPE" }))
            .cookie(&cookies["RATES"]),
    )
    .await
    .assert_refused(400, DOCUMENTS_NOT_READABLE, "RATES");
    desk.finish().await;
}

// ── the payload ──────────────────────────────────────────────────────────────

/// v2 finds the exchange with `row[0] === input.ref`: the number as text,
/// exactly. Anything else — no `ref`, another key, another type, a number that
/// would read the same after a conversion, a space, another case — is
/// `ไม่พบเอกสารรายการ`, and never a server error.
#[actix_web::test]
async fn the_ref_is_compared_as_v2_compares_it() {
    let fixture = Fixture::load(DAY);
    let replay = the_recorded_day(&fixture).await;

    // the control: the number itself is found
    let found = documents(&replay, "FR1", json!({ "ref": CASH })).await;
    assert_eq!(found.status, 200, "{:.300}", found.text);

    let not_found = [
        ("no payload key at all", json!({})),
        ("null", json!({ "ref": null })),
        ("the empty text", json!({ "ref": "" })),
        ("a number", json!({ "ref": 20261006 })),
        ("true", json!({ "ref": true })),
        // JavaScript's String([x]) is x: a conversion would find it
        ("a list of the number", json!({ "ref": [CASH] })),
        ("an object", json!({ "ref": { "ref": CASH } })),
        ("a space before", json!({ "ref": format!(" {CASH}") })),
        ("a space after", json!({ "ref": format!("{CASH} ") })),
        ("lower case", json!({ "ref": CASH.to_lowercase() })),
        ("the beginning of it", json!({ "ref": "XC-20261006-00000" })),
        ("a pattern", json!({ "ref": "XC-20261006-%" })),
        ("a NUL inside", json!({ "ref": format!("{CASH}\u{0}") })),
        (
            "the key the save answers with",
            json!({ "documentRef": CASH }),
        ),
        ("the key in another case", json!({ "Ref": CASH })),
        (
            "the key the page uses",
            json!({ "id": CASH, "receiptNo": CASH }),
        ),
        // a DEMO number has no documents (v2 looks in the LIVE book only; decision A2: no save made this one)
        (
            "a DEMO number no save made (decision A2)",
            json!({ "ref": DEMO }),
        ),
    ];
    for (what, payload) in not_found {
        let answer = documents(&replay, "FR1", payload).await;
        answer.assert_refused(400, NO_DOCUMENTS, what);
        assert_eq!(answer.header("cache-control"), Some("no-store"), "{what}");
    }

    // what else the payload carries changes nothing
    let extra = documents(
        &replay,
        "FR1",
        json!({ "ref": CASH, "staff": "somebody", "mode": "DEMO", "links": [] }),
    )
    .await;
    assert_eq!(extra.text, found.text);
    replay.finish().await;
}

/// An employee reads the exchanges of the own branch only, whatever branch the
/// payload names (v2's bridge overwrites `branchId` with the employee's).
#[actix_web::test]
async fn an_employee_reads_only_the_exchanges_of_the_own_branch() {
    let fixture = Fixture::load(DAY);
    let replay = the_recorded_day(&fixture).await;
    replay.desk.db.seed_branch("BR-2", "Second").await;
    let other = front_of("BR-2", 0xb2);
    replay.desk.employ(std::slice::from_ref(&other)).await;
    let cookies = sign_in(&replay.app, ORIGIN, std::slice::from_ref(&other)).await;

    for payload in [
        json!({ "ref": CASH }),
        json!({ "ref": CASH, "branchId": MAIN }),
    ] {
        replay
            .app
            .send(
                Call::action(ORIGIN, STAFF_RPC, "documents", payload.clone())
                    .cookie(&cookies[&other.key]),
            )
            .await
            .assert_refused(400, NO_DOCUMENTS, &format!("a Front of BR-2: {payload}"));
    }
    // and a Front of MAIN is not sent elsewhere by naming another branch
    for branch in ["BR-2", "NOWHERE", "MAIN_1", ""] {
        let answer = documents(&replay, "FR1", json!({ "ref": CASH, "branchId": branch })).await;
        assert_eq!(
            answer.status, 200,
            "branchId {branch:?}: {:.300}",
            answer.text
        );
        assert_eq!(answer.text, v2_text(&replay, 51), "branchId {branch:?}");
    }
    replay.finish().await;
}

// ── the family itself ────────────────────────────────────────────────────────

/// Of the families the server registers, one answers `documents` — on the
/// staff route only (v2 knows the name nowhere else) — and it answers no other
/// name on either route.
#[actix_web::test]
async fn the_family_answers_documents_on_the_staff_route_and_nothing_else() {
    let desk = Desk::open().await;
    let actions = registered_actions(&desk.state());
    let on = |route: RpcRoute, action: &str| {
        actions
            .iter()
            .filter(|family| family.handles(route, action))
            .count()
    };
    assert_eq!(on(RpcRoute::StaffRpc, "documents"), 1);
    assert_eq!(
        on(RpcRoute::Rpc, "documents"),
        0,
        "not an action of /api/rpc"
    );

    let family = actions
        .iter()
        .find(|family| family.handles(RpcRoute::StaffRpc, "documents"))
        .expect("the documents family");
    for route in [RpcRoute::Rpc, RpcRoute::StaffRpc] {
        for action in [
            "",
            "document",
            "Documents",
            "DOCUMENTS",
            "documents ",
            " documents",
            "documentsOf",
            "adminDocuments",
            "saveTransaction",
            "opsClock",
            "opsDocuments",
            "adminSnapshot",
            "getConnectionStatus",
        ] {
            assert!(!family.handles(route, action), "{action:?} on {route:?}");
        }
    }
    desk.finish().await;
}

/// What the family itself answers is what the owner would read: `/files/…`
/// links, `ref` first. The staff paths are the dispatcher's to write (it does
/// so for every answer of the staff route), and the branch is the one the
/// dispatcher resolved — not one the payload names.
#[actix_web::test]
async fn the_family_answers_the_owners_links_for_the_branch_it_is_given() {
    let fixture = Fixture::load(DAY);
    let replay = the_recorded_day(&fixture).await;
    replay.desk.db.seed_branch("BR-2", "Second").await;
    let actions = registered_actions(&replay.desk.state());
    let family = actions
        .iter()
        .find(|family| family.handles(RpcRoute::StaffRpc, "documents"))
        .expect("the documents family");

    let front = fixture.person("FR1").expect("FR1");
    let caller = RpcCaller {
        actor: actor_of(front),
        username: front.username.clone(),
        name: front.name.clone(),
    };
    let call = |payload: Value, branch: &'static str| {
        let payload: Map<String, Value> = payload.as_object().expect("an object").clone();
        (payload, BranchId::new(branch))
    };

    let (payload, branch) = call(json!({ "ref": CASH }), MAIN);
    let answer = family
        .run(RpcCall {
            route: RpcRoute::StaffRpc,
            action: "documents",
            payload,
            caller: &caller,
            branch: &branch,
            now: time("2026-10-06T05:22:00.000Z"),
        })
        .await
        .expect("the exchange of #10");
    assert_eq!(
        answer.to_text(),
        v2_text(&replay, 51).replace("/api/staff/file/", "/files/"),
        "what /api/rpc would print"
    );

    // the same number asked for another branch, the payload naming MAIN
    let (payload, branch) = call(json!({ "ref": CASH, "branchId": MAIN }), "BR-2");
    let refused = family
        .run(RpcCall {
            route: RpcRoute::StaffRpc,
            action: "documents",
            payload,
            caller: &caller,
            branch: &branch,
            now: time("2026-10-06T05:22:00.000Z"),
        })
        .await;
    match refused {
        Err(RpcFailure::Refused(refusal)) => {
            assert_eq!(refusal.status, 400);
            assert_eq!(refusal.sentence, NO_DOCUMENTS);
            assert!(refusal.no_store);
        }
        other => panic!("an exchange of MAIN asked for BR-2: {other:?}"),
    }
    replay.finish().await;
}

/// Something breaking on this side is v2's 500 sentence and says nothing of
/// the cause; it is not `ไม่พบเอกสารรายการ`.
#[actix_web::test]
async fn a_database_failure_is_the_server_error_sentence() {
    let fixture = Fixture::load(DAY);
    let replay = the_recorded_day(&fixture).await;
    sqlx::query("ALTER TABLE transactions RENAME TO transactions_gone")
        .execute(&replay.desk.db.pool)
        .await
        .expect("break the transactions table");

    let answer = documents(&replay, "FR1", json!({ "ref": CASH })).await;
    assert_eq!(answer.status, 500, "{:.300}", answer.text);
    assert_eq!(answer.json(), json!({ "error": SERVER_ERROR }));
    assert!(
        !answer.text.contains("transactions"),
        "the cause is not told"
    );
    // the failed test would leave the schema behind; put the table back
    sqlx::query("ALTER TABLE transactions_gone RENAME TO transactions")
        .execute(&replay.desk.db.pool)
        .await
        .expect("mend the transactions table");
    replay.finish().await;
}
