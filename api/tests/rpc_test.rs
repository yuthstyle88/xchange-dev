//! Written before the implementation (ticket T11a part one). Do not edit, delete or weaken.
//!
//! `POST /api/rpc` and `POST /api/staff/rpc` — the two doors of the unchanged
//! v2 screens — against what v2 recorded, **through HTTP**, with the real
//! sessions of ticket T4c.
//!
//! Part one dispatches ten actions (`rpc_common::PART_ONE`). What is replayed:
//!
//! - `rpc/transport.json`: every step;
//! - `rpc/engine_actions.json`: every step of the ten actions, and every step
//!   of a part-two name that `Policy` answers;
//! - `snapshot/empty.json`, `rates.json`, `shift.json`, `branches.json`: every
//!   step, every `view` for every recorded caller on both routes — the parity
//!   test;
//! - `rpc/index.json`: the role table.
//!
//! Each replay asserts how many steps it held to v2, how many to a decided
//! difference and which it left out, so a step cannot be skipped silently.
//!
//! Since T11a-0 the names of part two are counted per family
//! (`rpc_common::Family`, one `rpc_common/gap_*.rs` each): the tallies below
//! are computed from the three `WIRED` constants. With none wired
//! every number, name and answer is what it was before. (Those edits are the
//! lane controller's, made on ticket T11a-0.)
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Re-expected for RE-7 #604 by the ticket author (design `docs/design/blueprint/rates.md` §8
//! migration 6, §9): `adminSaveRate` is gone. Every recorded `adminSaveRate` step is a known
//! difference (design §9 removes the action, `Rule::Retired`): held to the answer of a name v2
//! does not know, and counted with the differences; an accepted one's rate is published behind
//! the routes. `engine_actions.json#150` asks after the id of #23, a direct save, and is held to
//! `CANCELLED` (`Rule::ResolvedAs`). The role table holds `adminSaveRate` to the unknown-name
//! answer for every role the index recorded. The database failure breaks `shifts`, not `rates`.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use chrono::{Duration, Utc};
use compat_common::{Fixture, Index, Person};
use rpc_common::{
    Answer, App, BAD_BRANCH, Call, Cookies, Desk, EMPLOYEE_AUTH_REQUIRED, FRONT_ONLY, Family,
    INVALID_REQUEST, MAIN, NO_SUCH_BRANCH, PART_ONE, RETIRED, ROLE_NOT_ALLOWED_RPC,
    ROLE_NOT_ALLOWED_STAFF, RPC, Replay, Rule, SERVER_ERROR, SIGN_IN, STAFF_RPC, USE_CUSTODY,
    V2_HEADERS_FAULT, one_of_each_role, retired_by_ca10, route_rule, sign_in, unknown_name_for,
};
use serde_json::{Value, json};
use snapshot_common::time;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::application::compat::envelope::RpcRoute;
use xchange::presentation::http::rpc::registered_actions;

const ORIGIN: &str = "http://desk.test";

/// True while no family of part two is wired: every tally is then the number
/// it was when part one was written.
const NO_FAMILY_WIRED: bool = !rpc_common::gap_documents::WIRED && !rpc_common::gap_save::WIRED;

/// The names the gap is held with, family by family in the order asked, and
/// how many (caller, route) pairs `Policy` lets through for them:
/// `saveTransaction` — the owner and FRONT, BACKEND, MANAGER, RATES on `/api/rpc`,
/// FRONT, BACKEND, MANAGER, RATES on the staff route (9); `documents` — eight
/// roles on the staff route (8). (CA-10 #626 retires the `ops*` custody family, design §6.)
const GAP_NAMES: [(Family, &[&str], usize); 2] = [
    (Family::Save, &["saveTransaction"], 9),
    (Family::Documents, &["documents"], 8),
];

/// The steps of `rpc/transport.json` v2 answers in the JavaScript engine's own
/// English (a body that is not JSON, or is `null`). Decided: v3 answers
/// `คำขอไม่ถูกต้อง`.
const ENGINE_SENTENCES: [usize; 8] = [18, 19, 20, 21, 43, 67, 68, 69];

fn numbers(of: &[(usize, &'static str)]) -> Vec<usize> {
    of.iter().map(|(n, _)| *n).collect()
}

async fn replay_whole(name: &str) -> (Fixture, Vec<usize>, rpc_common::Tally) {
    let fixture = Fixture::load(name);
    // LG-6.r5 (#1044, BE controller's ruling (a)): only `snapshot/rates.json` comes here, and it opens no
    // shift, so MAIN's ledger SAFE is never funded and `operations.safe` stays v2's null (a funded-then-swept
    // SAFE prints zeros).
    let desk = Desk::open_without_cash().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture).await;
    replay.run().await;
    let views: Vec<usize> = fixture
        .steps()
        .iter()
        .filter(|s| s.view.is_some())
        .map(|s| s.n)
        .collect();
    let tally = replay.tally.clone();
    replay.finish().await;
    (fixture, views, tally)
}

// ── rpc/transport.json ───────────────────────────────────────────────────────

/// Every step of `rpc/transport.json` through HTTP: status, headers, body, on
/// both routes, for the owner, an employee, an employee whose first password
/// is unchanged, and nobody.
#[actix_web::test]
async fn transport_json_is_answered_as_v2_answered_it() {
    let fixture = Fixture::load("rpc/transport.json");
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture).await;
    for n in ENGINE_SENTENCES {
        // sanity: these really are the engine's sentences (ASCII), not Thai
        assert!(
            fixture.step(n).error().is_some_and(str::is_ascii),
            "transport#{n} is not one of the JavaScript engine's sentences"
        );
        replay = replay.rule(
            n,
            Rule::Differs {
                status: 400,
                sentence: INVALID_REQUEST,
            },
        );
    }
    let mut replay = replay
        .rule(
            93,
            Rule::Excluded("`/api/staff/<other>` is not a route of this ticket"),
        )
        .rule(
            94,
            Rule::Excluded("the customer host: v3 does not tell hosts apart yet"),
        );
    // `opsNope` on the staff route (#84) reached custody in v2. CA-10 #626 (design §6) retires
    // every `ops*` name: v3 answers it as a name v2 does not know (`Rule::Retired`).
    replay.run().await;

    // the answers of the one action here are v2's bytes, key order included
    let mut same_text = 0;
    for step in fixture.steps() {
        if step.status == Some(200) {
            assert_eq!(
                Some(&replay.answers[&step.n].text),
                fixture.body_text(step.n).as_ref(),
                "transport#{}: the body, byte for byte",
                step.n
            );
            same_text += 1;
        }
    }
    assert_eq!(same_text, 24, "answers compared as text");

    // #95-#99 since ticket #146: the owner with a rejected origin
    assert_eq!(fixture.steps().len(), 99);
    assert_eq!(
        replay.tally.differences,
        // CA-10 #626: `opsNope` on `/api/rpc` (#36) is retired too
        {
            let mut all = [ENGINE_SENTENCES.to_vec(), vec![36, 84]].concat();
            all.sort_unstable();
            all
        }
    );
    assert!(replay.tally.part_two.is_empty());
    assert_eq!(numbers(&replay.tally.excluded), vec![93, 94]);
    // 88 since ticket #146 added #95-#99; 87 since CA-10 #626 retired #36
    assert_eq!(replay.tally.replayed.len(), 87);
    assert!(replay.tally.applied_behind.is_empty());
    replay.finish().await;
}

/// What the recording does not hold about a body: a byte-order mark is dropped
/// and bytes that are not UTF-8 are replaced, as `request.text()` did in v2;
/// and a body no 120,000 units could fill is refused in the route's sentence
/// without being read.
#[actix_web::test]
async fn a_body_is_read_as_v2_read_it() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    let ok = r#"{"ready":true,"sheetUrl":"/#transactions","folderUrl":"/documents"}"#;
    let ok_staff = r#"{"ready":true,"sheetUrl":"/staff","folderUrl":"/documents"}"#;
    let request = r#"{"action":"getConnectionStatus","payload":{}}"#;

    for (path, who, expected) in [
        (RPC, "OWNER", ok),
        (RPC, "FRONT", ok),
        (STAFF_RPC, "FRONT", ok_staff),
    ] {
        let label = format!("{path} as {who}");
        let with_mark = [b"\xEF\xBB\xBF".as_slice(), request.as_bytes()].concat();
        let answer = app
            .send(Call::post(ORIGIN, path, with_mark).cookie(&cookies[who]))
            .await;
        assert_eq!(
            answer.status, 200,
            "{label}: a byte-order mark: {}",
            answer.text
        );
        assert_eq!(answer.text, expected, "{label}");

        let mut not_utf8 = br#"{"action":"getConnectionStatus","payload":{"pad":""#.to_vec();
        not_utf8.extend_from_slice(&[0xff, 0xfe]);
        not_utf8.extend_from_slice(br#""}}"#);
        let answer = app
            .send(Call::post(ORIGIN, path, not_utf8).cookie(&cookies[who]))
            .await;
        assert_eq!(
            answer.status, 200,
            "{label}: bytes that are not UTF-8: {}",
            answer.text
        );
        assert_eq!(answer.text, expected, "{label}");

        // 400,000 bytes: more than 120,000 units of anything
        let mut huge = br#"{"action":"getConnectionStatus","payload":{"pad":""#.to_vec();
        huge.resize(400_000, b'x');
        huge.extend_from_slice(br#""}}"#);
        let answer = app
            .send(Call::post(ORIGIN, path, huge).cookie(&cookies[who]))
            .await;
        let sentence = if path == RPC {
            "ข้อมูลคำขอใหญ่เกินกำหนด"
        } else {
            "ข้อมูลมากเกินไป"
        };
        answer.assert_refused(400, sentence, &label);
        assert_eq!(answer.header("cache-control"), Some("no-store"), "{label}");
    }

    // nobody signed in is told so before the size of the body is looked at
    let mut huge = b"{".to_vec();
    huge.resize(400_000, b' ');
    let answer = app.send(Call::post(ORIGIN, RPC, huge)).await;
    answer.assert_refused(401, SIGN_IN, "nobody, a huge body");
    assert_eq!(answer.header("cache-control"), None);
    desk.finish().await;
}

/// The order of the checks on each route, where one request fails two of
/// them: which sentence is said tells which check came first.
///
/// `/api/rpc` (v2 `server.mjs`, `rpc()`): the session, the origin, the size,
/// the JSON, the name against the role, the payload. `/api/staff/rpc` (the
/// staff route and its bridge): the origin, the size, the JSON, the session,
/// the first password, the name against the role.
#[actix_web::test]
async fn each_route_makes_its_checks_in_v2s_order() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut people = one_of_each_role();
    people.push(Person {
        key: "NEW".to_string(),
        kind: "staff".to_string(),
        id: "00000000-0000-4000-8000-0000000000cc".to_string(),
        username: "new".to_string(),
        name: "NEW Front".to_string(),
        role: "FRONT".to_string(),
        branch_id: Some(MAIN.to_string()),
        must_change: true,
    });
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    let too_long = format!(
        r#"{{"action":"getConnectionStatus","payload":{{"pad":"{}"}}}}"#,
        "x".repeat(120_000)
    );

    // /api/rpc: the name is judged before the payload
    app.send(
        Call::post(
            ORIGIN,
            RPC,
            r#"{"action":"adminCreateBranch","payload":null}"#,
        )
        .cookie(&cookies["FRONT"]),
    )
    .await
    .assert_refused(
        400,
        ROLE_NOT_ALLOWED_RPC,
        "a name the role may not use, a payload of null",
    );
    app.send(
        Call::post(ORIGIN, RPC, r#"{"action":"adminSnapshot","payload":null}"#)
            .cookie(&cookies["FRONT"]),
    )
    .await
    .assert_refused(
        400,
        INVALID_REQUEST,
        "a name the role may use, a payload of null",
    );
    // /api/rpc: nobody is told to sign in whatever else is wrong
    for call in [
        Call::post(ORIGIN, RPC, "not json").without("origin"),
        Call::post(ORIGIN, RPC, too_long.clone()),
        Call::post(ORIGIN, RPC, "not json").cookie(&cookies["NEW"]),
    ] {
        let answer = app.send(call).await;
        answer.assert_refused(401, SIGN_IN, "nobody on /api/rpc");
        assert_eq!(answer.header("cache-control"), None);
    }
    // /api/rpc: the origin before the size, the size before the JSON
    app.send(
        Call::post(ORIGIN, RPC, too_long.clone())
            .without("origin")
            .cookie(&cookies["OWNER"]),
    )
    .await
    // ticket #146: the owner is told v2's ORIGIN_REJECTED (rpc/transport.json #95)
    .assert_refused(400, "ORIGIN_REJECTED", "no origin, too long");
    app.send(Call::post(ORIGIN, RPC, format!("{too_long} not json")).cookie(&cookies["OWNER"]))
        .await
        .assert_refused(400, "ข้อมูลคำขอใหญ่เกินกำหนด", "too long, not JSON");

    // /api/staff/rpc: the origin first
    app.send(Call::post(ORIGIN, STAFF_RPC, too_long.clone()).without("x-xchange-request"))
        .await
        .assert_refused(400, INVALID_REQUEST, "nobody, no marker, too long");
    // then the body, before anybody is asked who they are
    app.send(Call::post(ORIGIN, STAFF_RPC, too_long.clone()))
        .await
        .assert_refused(400, "ข้อมูลมากเกินไป", "nobody, too long");
    app.send(Call::post(ORIGIN, STAFF_RPC, "not json"))
        .await
        .assert_refused(400, INVALID_REQUEST, "nobody, not JSON");
    // then the session
    app.send(Call::post(ORIGIN, STAFF_RPC, r#"{"action":"nope"}"#))
        .await
        .assert_refused(401, EMPLOYEE_AUTH_REQUIRED, "nobody, a name nobody may use");
    // then the first password, before the name
    app.send(Call::post(ORIGIN, STAFF_RPC, r#"{"action":"nope"}"#).cookie(&cookies["NEW"]))
        .await
        .assert_refused(
            400,
            "กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน",
            "the first password unchanged, a name nobody may use",
        );
    // then the name
    app.send(Call::post(ORIGIN, STAFF_RPC, r#"{"action":"nope"}"#).cookie(&cookies["FRONT"]))
        .await
        .assert_refused(400, ROLE_NOT_ALLOWED_STAFF, "a name nobody may use");
    app.send(
        Call::post(
            ORIGIN,
            STAFF_RPC,
            r#"{"action":"adminDeskConfig","payload":null}"#,
        )
        .cookie(&cookies["FINANCE"]),
    )
    .await
    .assert_refused(400, FRONT_ONLY, "the desk config, not a Front");
    desk.finish().await;
}

/// A browser can hold an owner session and a staff session at once:
/// `/api/rpc` answers the owner, `/api/staff/rpc` the employee.
#[actix_web::test]
async fn with_both_sessions_each_route_answers_its_own_caller() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    let both = format!("{}; {}", cookies["FRONT"], cookies["OWNER"]);

    let on_rpc = app
        .send(Call::action(ORIGIN, RPC, "adminSnapshot", json!({})).cookie(&both))
        .await;
    assert_eq!(on_rpc.status, 200, "{:.300}", on_rpc.text);
    assert_eq!(on_rpc.json()["actor"], "owner");
    assert!(on_rpc.json().get("employee").is_none());

    let on_staff = app
        .send(Call::action(ORIGIN, STAFF_RPC, "adminSnapshot", json!({})).cookie(&both))
        .await;
    assert_eq!(on_staff.status, 200, "{:.300}", on_staff.text);
    assert_eq!(on_staff.json()["actor"], "role0");
    assert_eq!(on_staff.json()["employee"]["role"], "FRONT");

    // the owner alone has no session on the staff route
    let owner_only = app
        .send(Call::action(ORIGIN, STAFF_RPC, "adminSnapshot", json!({})).cookie(&cookies["OWNER"]))
        .await;
    owner_only.assert_refused(401, EMPLOYEE_AUTH_REQUIRED, "the owner on the staff route");
    desk.finish().await;
}

/// The clock is the system's unless a test registers one.
#[actix_web::test]
async fn without_a_registered_clock_the_routes_read_the_system_clock() {
    let desk = Desk::open().await;
    let state = desk.state();
    let app = App::new(
        actix_web::test::init_service(
            actix_web::App::new().configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
        )
        .await,
        |t| t.to_request(),
    );
    let owner = one_of_each_role().remove(0);
    let cookies = sign_in(&app, ORIGIN, &[owner]).await;
    let before = Utc::now() - Duration::seconds(5);
    let answer = app
        .send(Call::action(ORIGIN, RPC, "adminSnapshot", json!({})).cookie(&cookies["OWNER"]))
        .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    let synced = time(answer.json()["syncedAt"].as_str().expect("syncedAt"));
    assert!(
        synced >= before && synced <= Utc::now() + Duration::seconds(5),
        "syncedAt {synced} is the system clock"
    );
    desk.finish().await;
}

/// Something breaking on this side is v2's 500 sentence and says nothing of
/// the cause; it is never a refusal.
#[actix_web::test]
async fn a_database_failure_is_the_server_error_sentence() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    // re-expected for RE-7 #604: `rates` is dropped; the snapshot also reads `shifts`
    sqlx::query("ALTER TABLE shifts RENAME TO shifts_gone")
        .execute(&desk.db.pool)
        .await
        .expect("break the shifts table");

    for (path, who) in [(RPC, "OWNER"), (STAFF_RPC, "FRONT")] {
        let answer = app
            .send(Call::action(ORIGIN, path, "adminSnapshot", json!({})).cookie(&cookies[who]))
            .await;
        assert_eq!(answer.status, 500, "{path}: {:.300}", answer.text);
        assert_eq!(answer.json(), json!({ "error": SERVER_ERROR }), "{path}");
        assert!(
            !answer.text.contains("shifts"),
            "{path}: the cause is not told"
        );
    }
    desk.finish().await;
}

// ── rpc/engine_actions.json ──────────────────────────────────────────────────

/// `rpc/engine_actions.json` through HTTP, step by step: every step of the ten
/// actions equals v2's answer (ids made by the server paired; `row`,
/// `startRow`, `endRow`, `rawStatus` left out), every refusal in status and
/// sentence.
///
/// EXCLUDED from the comparison, because part one does not dispatch them
/// (ticket T11a part two): the 41 `saveTransaction` steps and the 2 `documents`
/// steps that `Policy` lets through. Each of them is still sent and held to
/// the unknown-name refusal; the three accepted saves are then applied through
/// the save workflow behind the routes (with the three uploads before them),
/// so that the close of the shift and the snapshots after them see what v2 saw.
#[actix_web::test]
async fn engine_actions_json_is_answered_as_v2_answered_it() {
    let fixture = Fixture::load("rpc/engine_actions.json");
    assert_eq!(fixture.step(159).error(), Some(V2_HEADERS_FAULT));
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture)
        .await
        // decided: the same request again from another branch answers the new
        // branch's snapshot, as #160 does from the new branch itself
        .rule(159, Rule::AnswersAs(160))
        // re-expected for RE-7 #604 (known difference: design §9 removes the action): #150
        // asks after the id of #23, a direct save, which v3 no longer applies
        .rule(150, Rule::ResolvedAs("CANCELLED"))
        // re-expected for CA-10 #626 (design §6 retires the direct shift actions): #151 asks after
        // the id of #40, a direct shift open, which v3 no longer applies under that id
        .rule(151, Rule::ResolvedAs("CANCELLED"));
    replay.run().await;

    assert_eq!(fixture.steps().len(), 184);
    // re-expected for RE-7 #604: the 29 `adminSaveRate` steps (#4–#31, #148) and #150
    let retired: Vec<usize> = (4..=31).chain([148]).collect();
    assert_eq!(retired.len(), 29);
    // RBAC §11.2 (#3659): the owner holds ExchangeTransact again via EXECUTIVE (Customers Full), so
    // #124 (the owner's repeated reserveCustomer) is replayed through HTTP and hits CI-4's
    // SameCustomerAs rule.
    let mut differences: Vec<usize> = [124, 150, 151, 159].to_vec();
    differences.extend(&retired);
    // re-expected for CA-10 #626: the 49 direct shift steps (#32–#67, #132–#143, #149)
    let ca10: Vec<usize> = (32..=67).chain(132..=143).chain([149]).collect();
    assert_eq!(ca10.len(), 49);
    differences.extend(&ca10);
    differences.sort_unstable();
    let mut tallied = replay.tally.differences.clone();
    tallied.sort_unstable();
    assert_eq!(tallied, differences);
    assert_eq!(numbers(&replay.tally.excluded), vec![82, 83, 109]);
    let saves: Vec<usize> = (84..=108).chain(110..=123).chain([125, 144]).collect();
    let documents = [127, 128];
    assert_eq!(saves.len() + documents.len(), 43);
    // of those 43, the steps of a family that is not wired yet are part two
    let mut part_two = Vec::new();
    if !Family::Save.wired() {
        part_two.extend(saves.iter().copied());
    }
    if !Family::Documents.wired() {
        part_two.extend(documents.iter().copied());
    }
    part_two.sort_unstable();
    assert_eq!(replay.tally.part_two, part_two);
    if NO_FAMILY_WIRED {
        assert_eq!(part_two.len(), 43);
    }
    // RBAC §11.2 (#3659): the owner holds ExchangeTransact again via EXECUTIVE (Customers Full), so
    // no step here is refused by the Blueprint where v2 let it through, #81, #126 (BE1) and #129
    // (RT1) are widened where v2 refused them by role, and the owner's 8 saves (#111, #112, #114,
    // #115, #120–#123) meet the CA-10 teller gate (`owner_no_open_shift`).
    let owners_c2_c3: Vec<usize> = Vec::new();
    assert_eq!(replay.tally.blueprint, owners_c2_c3);
    assert_eq!(replay.tally.widened, vec![81, 126, 129]);
    assert_eq!(
        replay.tally.owner_no_open_shift,
        vec![111, 112, 114, 115, 120, 121, 122, 123]
    );
    assert_eq!(
        replay.tally.replayed.len(),
        107 - 50 - 1 + (43 - part_two.len())
            - owners_c2_c3.len()
            - replay.tally.widened.len()
            - replay.tally.owner_no_open_shift.len()
    );
    // the three uploads, the three accepted owner saves (one of them sent twice),
    // and (RE-7 #604) the five accepted direct saves, published behind.
    // Re-expected for TX-6 #674 by the test-writer (decision A2, owner 2026-10-11, design
    // quote-transaction.md §4.6.1): #125, the DEMO save at rate 1, is now the refusal
    // RATE_MISMATCH (fixture step 125), so nothing of it is applied behind the routes.
    // re-expected for CA-10 #626: the direct shift steps v2 accepted, applied behind the routes
    // (`Replay::retired_behind`): the open #40, the moves #63 and #66, the close #141; their repeats
    // (#41, #64, #142) apply nothing more
    let mut applied_behind = vec![
        23, 24, 25, 28, 29, 40, 63, 66, 82, 83, 109, 120, 121, 123, 141,
    ];
    applied_behind.sort_unstable();
    let mut tallied = replay.tally.applied_behind.clone();
    tallied.sort_unstable();
    assert_eq!(tallied, applied_behind);

    // every step of the ten actions was held to v2 or to the decided difference
    for step in fixture.steps() {
        let Some(action) = &step.action else { continue };
        if PART_ONE.contains(&action.as_str()) {
            assert!(
                replay.tally.replayed.contains(&step.n)
                    || [124, 150, 151, 159].contains(&step.n)
                    || retired.contains(&step.n)
                    // CA-10 #626: held to the unknown-name answer instead
                    || ca10.contains(&step.n)
                    // OR-4b: held to the Blueprint's refusal instead (C2, C3)
                    || replay.tally.blueprint.contains(&step.n)
                    // RBAC §11.2 (#3659): held to not be v2's role refusal
                    || replay.tally.widened.contains(&step.n),
                "engine_actions#{} ({action}) was not compared",
                step.n
            );
        }
    }
    // re-expected for RE-7 #604: no direct save asks the bank any more
    assert_eq!(replay.desk.bank.times_asked(), 0);
    replay.finish().await;
}

// ── the parity test: the four snapshot files that hold no exchange ──────────

/// `snapshot/empty.json`: every answer of every caller on both routes — and
/// here, where nothing was written yet, every accepted answer is v2's text
/// byte for byte: the order of the keys, the staff view's `employees` and
/// `employee`, what a RATES and a SYSTEM employee are shown.
#[actix_web::test]
async fn empty_json_is_v2s_text_for_every_caller_on_both_routes() {
    let fixture = Fixture::load("snapshot/empty.json");
    // LG-6.r5 (#1044, BE controller's ruling (a)): this recording opens no shift, so MAIN's ledger SAFE is
    // never funded and `operations.safe` stays v2's null (a funded-then-swept SAFE prints zeros).
    let desk = Desk::open_without_cash().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture).await;
    replay.run().await;

    assert_eq!(replay.tally.widened, vec![12, 16, 20]);
    assert!(replay.tally.owner_no_open_shift.is_empty());
    assert_eq!(replay.tally.replayed.len(), 45);
    assert_eq!(replay.tally.total(), 48);
    let mut same_text = 0;
    let mut callers = std::collections::BTreeSet::new();
    for step in fixture.steps() {
        if step.status != Some(200) {
            continue;
        }
        assert_eq!(
            Some(&replay.answers[&step.n].text),
            fixture.body_text(step.n).as_ref(),
            "empty#{} ({} as {}): the body, byte for byte",
            step.n,
            step.route.as_deref().unwrap_or_default(),
            step.sent_as
        );
        same_text += 1;
        callers.insert((
            step.sent_as.clone(),
            step.path().unwrap_or_default().to_string(),
        ));
    }
    assert_eq!(same_text, 37, "accepted answers compared as text");
    // the owner on /api/rpc, nine roles on each route
    assert_eq!(callers.len(), 19);
    replay.finish().await;
}

#[actix_web::test]
async fn rates_json_is_answered_as_v2_answered_it() {
    let (fixture, views, tally) = replay_whole("snapshot/rates.json").await;
    assert_eq!(fixture.steps().len(), 21);
    // re-expected for RE-7 #604: the five direct saves are known differences
    assert_eq!(tally.differences, vec![1, 2, 3, 6, 7]);
    assert_eq!(tally.widened, vec![17]);
    assert!(tally.owner_no_open_shift.is_empty());
    assert_eq!(tally.replayed.len(), 15);
    assert_eq!(views.len(), 16);
    assert!(
        views
            .iter()
            .all(|n| tally.replayed.contains(n) || tally.widened.contains(n))
    );
}

// Re-expected for CA-10 #626 by the ticket author: `snapshot/shift.json` recorded v2's direct shift
// actions (`adminOpenShift`, `adminMoveStock`, `adminCloseShift`), the subject its replay held. Design
// §6 retires them, replaced by counts and moves, which their own tests hold; the replay of this file
// retires with them. (`the_staff_route_keeps_the_order_of_the_keys` still reads it.)

/// `snapshot/branches.json`: three branches, employees of two of them. The one
/// decided difference: step 3, `adminCreateBranch` sent again from `MAIN`,
/// which v2 refuses with a sentence about its own tables — v3 answers the new
/// branch's snapshot, as step 4 does from the new branch.
#[actix_web::test]
async fn branches_json_is_answered_as_v2_answered_it() {
    let fixture = Fixture::load("snapshot/branches.json");
    assert_eq!(fixture.step(3).error(), Some(V2_HEADERS_FAULT));
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture)
        .await
        .rule(3, Rule::AnswersAs(4));
    replay.run().await;

    assert_eq!(fixture.steps().len(), 39);
    // CI-4 (#651): #10 is SameCustomerAs (K1, a customer per company); re-expected for RE-7
    // #604: the three direct saves (#1, #6, #7) are known differences; re-expected for CA-10
    // #626: the direct shift opens #8 and #37 too (design §6)
    assert_eq!(replay.tally.differences, vec![1, 3, 6, 7, 8, 10, 37]);
    // RBAC §11.2 (#3659): the owner holds ExchangeTransact again via EXECUTIVE (Customers Full), so
    // #9 is replayed; #28 (adminDeskConfig as BE2 on /api/staff/rpc) is widened.
    assert_eq!(replay.tally.blueprint, Vec::<usize>::new());
    assert_eq!(replay.tally.widened, vec![28]);
    assert!(replay.tally.owner_no_open_shift.is_empty());
    // CA-10 #626: #8 and #37 left the replayed for the differences
    assert_eq!(replay.tally.replayed.len(), 31);
    let views: Vec<usize> = fixture
        .steps()
        .iter()
        .filter(|s| s.view.is_some())
        .map(|s| s.n)
        .collect();
    assert_eq!(views.len(), 22);
    assert!(
        views
            .iter()
            .all(|n| replay.tally.replayed.contains(n) || replay.tally.widened.contains(n))
    );

    // the difference wrote nothing twice: still one airport
    let airports: i64 = sqlx::query_scalar("SELECT count(*) FROM branches WHERE name = 'Airport'")
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("count the airports");
    assert_eq!(airports, 1);
    replay.finish().await;
}

/// The staff route prints a snapshot's keys in v2's order, not sorted by name:
/// the screens show the 153-code objects in key order. (`staff_view` works on a
/// `serde_json::Value`, which forgets the order.)
#[actix_web::test]
async fn the_staff_route_keeps_the_order_of_the_keys() {
    let fixture = Fixture::load("snapshot/shift.json");
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture).await;
    for n in 1..=8 {
        replay.step(fixture.step(n)).await;
    }

    // step 7: FR1's snapshot on the staff route, a shift just opened
    let v2 = fixture.body_text(7).expect("a recorded body");
    let from = v2.find(r#""opening":{"#).expect("an opening in v2's text");
    let to = from + v2[from..].find('}').expect("the end of the opening") + 1;
    let opening = &v2[from..to];
    assert!(
        opening.starts_with(r#""opening":{"THB":50000,"USD":1000.5,"EUR":0,"GBP":0,"#),
        "{opening:.80}"
    );
    let actual = &replay.answers[&7].text;
    assert!(
        actual.contains(opening),
        "the staff route's `opening` is not in v2's order: {:.200}",
        &actual[actual.find(r#""opening""#).unwrap_or(0)..]
    );
    // and the top level: v2's keys in v2's order, then the staff view's two
    let keys: Vec<&str> = [
        "version",
        "rateHistory",
        "customers",
        "branchId",
        "branches",
        "transactions",
        "rates",
        "stock",
        "updated",
        "ready",
        "actor",
        "syncedAt",
        "sheetUrl",
        "deskUrl",
        "operations",
        "employees",
        "employee",
    ]
    .to_vec();
    let mut at = 0;
    for key in keys {
        let found = actual[at..]
            .find(&format!("\"{key}\":"))
            .unwrap_or_else(|| panic!("`{key}` does not come after the key before it"));
        at += found;
    }
    // step 8: the desk config on the staff route
    assert!(
        replay.answers[&8]
            .text
            .starts_with(r#"{"branchId":"MAIN","branchName":"XC-B","rates":["#)
    );
    replay.finish().await;
}

/// Who the staff view lists: the active employees of the caller's own branch,
/// oldest account first.
#[actix_web::test]
async fn the_staff_view_lists_the_active_employees_of_the_branch_in_the_order_made() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut people = one_of_each_role();
    // made last, sorts first by username; and somebody of another branch
    people.push(Person {
        key: "LATE".to_string(),
        kind: "staff".to_string(),
        id: "00000000-0000-4000-8000-0000000000aa".to_string(),
        username: "aaa".to_string(),
        name: "Made Last".to_string(),
        role: "FRONT".to_string(),
        branch_id: Some(MAIN.to_string()),
        must_change: false,
    });
    people.push(Person {
        key: "ELSEWHERE".to_string(),
        kind: "staff".to_string(),
        id: "00000000-0000-4000-8000-0000000000bb".to_string(),
        username: "bbb".to_string(),
        name: "Of Another Branch".to_string(),
        role: "FRONT".to_string(),
        branch_id: Some("B-elsewhere".to_string()),
        must_change: false,
    });
    desk.db.seed_branch("B-elsewhere", "Elsewhere").await;
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    sqlx::query("UPDATE employees SET active = false WHERE username = 'role1'")
        .execute(&desk.db.pool)
        .await
        .expect("close an account");

    let answer = app
        .send(Call::action(ORIGIN, STAFF_RPC, "adminSnapshot", json!({})).cookie(&cookies["FRONT"]))
        .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    let listed: Vec<String> = answer.json()["employees"]
        .as_array()
        .expect("employees")
        .iter()
        .map(|e| e["name"].as_str().expect("a name").to_string())
        .collect();
    let mut expected: Vec<String> = (0..9)
        .filter(|i| *i != 1)
        .map(|i| format!("Somebody {i}"))
        .collect();
    expected.push("Made Last".to_string());
    assert_eq!(listed, expected);
    desk.finish().await;
}

// ── rpc/index.json and Policy: who may ask what, where ───────────────────────

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

/// The refusals a route gives for who is asking, before any use case runs.
fn is_gate_refusal(answer: &Answer) -> bool {
    answer.status == 401
        || (answer.status == 400
            && [
                ROLE_NOT_ALLOWED_RPC,
                ROLE_NOT_ALLOWED_STAFF,
                FRONT_ONLY,
                INVALID_REQUEST,
            ]
            .contains(&answer.error().as_str()))
}

async fn ask(app: &App, cookies: &Cookies, path: &str, action: &str, who: &str) -> Answer {
    app.send(Call::action(ORIGIN, path, action, json!({})).cookie(&cookies[who]))
        .await
}

/// `rpc/index.json` — what the recordings show per action and route — and
/// `Policy` (#65) agree with what the routes answer, for the ten actions:
///
/// - every role a recorded step was accepted from gets past the gate;
/// - every recorded refusal for who is asking is answered to each role it was
///   said to, in status and sentence;
/// - every other recorded refusal is the business of a file replayed above;
/// - and for every one of the ten roles on both routes the gate is exactly
///   `Policy::allow` with that route's entry path.
#[actix_web::test]
async fn index_json_and_policy_agree_with_the_routes_for_the_ten_actions() {
    let index = Index::load();
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    let replayed_files = [
        "rpc/engine_actions.json",
        "snapshot/empty.json",
        "snapshot/rates.json",
        "snapshot/shift.json",
        "snapshot/branches.json",
    ];

    let mut accepted = 0;
    let mut blueprint_refusals = 0;
    let mut widened = 0;
    let mut gate_refusals = 0;
    let mut use_case_refusals = 0;
    let mut retired = 0;
    for action in PART_ONE {
        let routes = index
            .actions
            .get(action)
            .unwrap_or_else(|| panic!("the index has no {action}"));
        assert!(
            routes.contains_key(RPC),
            "{action} was recorded on /api/rpc"
        );
        // re-expected for CA-10 #626 by the ticket author: the direct shift actions too (design §6)
        if action == RETIRED || retired_by_ca10(action) {
            // re-expected for RE-7 #604, a known difference (design §9 removes the action):
            // every role the index recorded, accepted or refused, is answered as a name v2
            // does not know
            for (path, record) in routes {
                let roles = record
                    .accepted_from
                    .iter()
                    .chain(record.refusals.iter().flat_map(|r| r.from.iter()));
                for role in roles {
                    let answer = ask(&app, &cookies, path, action, role).await;
                    let unknown = ask(&app, &cookies, path, &unknown_name_for(action), role).await;
                    assert_eq!(
                        (answer.status, answer.text.as_str()),
                        (unknown.status, unknown.text.as_str()),
                        "{action} on {path} as {role}: answered as a name v2 does not know"
                    );
                    retired += 1;
                }
            }
            continue;
        }
        for (path, record) in routes {
            assert!(
                [RPC, STAFF_RPC].contains(&path.as_str()),
                "{action} on {path}"
            );
            for role in &record.accepted_from {
                let answer = ask(&app, &cookies, path, action, role).await;
                // OR-4b (#767): v2 accepted the owner here; the Blueprint refuses (C2, C3)
                let person = people.iter().find(|p| &p.key == role).expect("a person");
                let entry = if path == RPC {
                    EntryPath::Rpc
                } else {
                    EntryPath::StaffRpc
                };
                if let Err(refusal) = route_rule(Some(&actor_of(person)), entry, action) {
                    answer.assert_refused(
                        refusal.status(),
                        &refusal.to_string(),
                        &format!("{action} on {path} as {role}: C2, C3"),
                    );
                    blueprint_refusals += 1;
                    continue;
                }
                assert!(
                    !is_gate_refusal(&answer),
                    "{action} on {path}: v2 accepted {role}, v3 answers {} {:.200}",
                    answer.status,
                    answer.text
                );
                accepted += 1;
            }
            for refusal in &record.refusals {
                let at_the_gate = refusal.status == 401
                    || [ROLE_NOT_ALLOWED_RPC, ROLE_NOT_ALLOWED_STAFF, FRONT_ONLY]
                        .contains(&refusal.error.as_str());
                if at_the_gate {
                    for role in &refusal.from {
                        let answer = ask(&app, &cookies, path, action, role).await;
                        let person = people.iter().find(|p| &p.key == role).expect("a person");
                        let entry = if path == RPC {
                            EntryPath::Rpc
                        } else {
                            EntryPath::StaffRpc
                        };
                        if route_rule(Some(&actor_of(person)), entry, action).is_ok() {
                            assert!(
                                !is_gate_refusal(&answer),
                                "{action} on {path} as {role} ({}): widened by §11.2, got {} {:.200}",
                                refusal.first,
                                answer.status,
                                answer.text
                            );
                            widened += 1;
                            continue;
                        }
                        answer.assert_refused(
                            refusal.status,
                            &refusal.error,
                            &format!("{action} on {path} as {role} ({})", refusal.first),
                        );
                        gate_refusals += 1;
                    }
                } else if refusal.error == V2_HEADERS_FAULT {
                    // the decided difference, held in `branches_json_…` above
                    assert_eq!(refusal.first, "snapshot/branches.json#3");
                    use_case_refusals += 1;
                } else if refusal.error == USE_CUSTODY {
                    // rpc/ops_actions.json #18–#20: the custody guard, below
                    assert!(refusal.first.starts_with("rpc/ops_actions.json#"));
                    use_case_refusals += 1;
                } else {
                    let (file, _) = refusal.first.split_once('#').expect("file#step");
                    assert!(
                        replayed_files.contains(&file),
                        "{action} on {path}: `{}` ({}) is recorded in a file this test does not replay",
                        refusal.error,
                        refusal.first
                    );
                    use_case_refusals += 1;
                }
            }
        }
    }
    assert_eq!(
        (
            accepted,
            blueprint_refusals,
            widened,
            gate_refusals,
            use_case_refusals,
            retired
        ),
        // RBAC §11.2 (#3659): the owner holds ExchangeTransact again via EXECUTIVE (Customers Full),
        // and BACKEND, MANAGER, RATES are widened on ExchangeTransact.
        (38, 0, 4, 7, 16, 49),
        "what the index holds for the ten actions"
    );

    // the gate is Policy's, with the route's own entry path
    let mut asked = 0;
    for person in &people {
        let actor = actor_of(person);
        for (path, entry) in [(RPC, EntryPath::Rpc), (STAFF_RPC, EntryPath::StaffRpc)] {
            for action in PART_ONE {
                let answer = ask(&app, &cookies, path, action, &person.key).await;
                let label = format!("{action} on {path} as {}", person.key);
                // OR-4b (#767): the gate is the route table's rule (`route_rule`). The staff route
                // reads the staff session only: an owner's session is nobody there (C1 admits the
                // owner's user on a staff path only through a session of that path)
                let caller = (path == RPC || !person.is_owner()).then_some(&actor);
                match route_rule(caller, entry, action) {
                    Ok(()) => assert!(
                        !is_gate_refusal(&answer),
                        "{label}: Policy allows, the route answers {} {:.200}",
                        answer.status,
                        answer.text
                    ),
                    Err(refusal) => {
                        answer.assert_refused(refusal.status(), &refusal.to_string(), &label);
                        assert_eq!(answer.header("cache-control"), Some("no-store"), "{label}");
                    }
                }
                asked += 1;
            }
        }
    }
    assert_eq!(asked, 200);
    desk.finish().await;
}

/// The branch a request is answered for: the owner's `branchId` (which must
/// look like one and must exist), an employee's own whatever is named.
#[actix_web::test]
async fn the_branch_is_the_owners_choice_and_an_employees_own() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    desk.db.seed_branch("B-two", "Two").await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    let snapshot = |path: &'static str, who: &'static str, payload: Value| {
        let (app, cookies) = (&app, &cookies);
        async move {
            app.send(Call::action(ORIGIN, path, "adminSnapshot", payload).cookie(&cookies[who]))
                .await
        }
    };

    for (payload, branch) in [
        (json!({}), MAIN),
        (json!({ "branchId": null }), MAIN),
        (json!({ "branchId": "" }), MAIN),
        (json!({ "branchId": "B-two" }), "B-two"),
    ] {
        let answer = snapshot(RPC, "OWNER", payload.clone()).await;
        assert_eq!(answer.status, 200, "{payload}: {:.200}", answer.text);
        assert_eq!(answer.json()["branchId"], branch, "{payload}");
        assert_eq!(
            answer.json()["deskUrl"],
            format!("/desk?branch={branch}"),
            "{payload}"
        );
    }
    for (payload, sentence) in [
        (json!({ "branchId": "not a branch!" }), BAD_BRANCH),
        (json!({ "branchId": "x".repeat(71) }), BAD_BRANCH),
        (json!({ "branchId": { "id": "MAIN" } }), BAD_BRANCH),
        (json!({ "branchId": "B-nowhere" }), NO_SUCH_BRANCH),
        (json!({ "branchId": "main" }), NO_SUCH_BRANCH),
    ] {
        snapshot(RPC, "OWNER", payload.clone())
            .await
            .assert_refused(400, sentence, &payload.to_string());
    }
    // an employee is answered for their own branch on both routes
    for path in [RPC, STAFF_RPC] {
        for payload in [
            json!({ "branchId": "B-two" }),
            json!({ "branchId": "not a branch!" }),
            json!({ "branchId": "B-nowhere" }),
        ] {
            let answer = snapshot(path, "FRONT", payload.clone()).await;
            assert_eq!(answer.status, 200, "{path} {payload}: {:.200}", answer.text);
            assert_eq!(answer.json()["branchId"], MAIN, "{path} {payload}");
        }
    }
    // a write lands in the branch named, and nowhere else
    let saved = app
        .send(
            Call::action(
                ORIGIN,
                RPC,
                "reserveCustomer",
                json!({ "branchId": "B-two", "name": "A Customer", "passportNo": "P7000001",
                        "nationality": "DEU", "mode": "LIVE" }),
            )
            .cookie(&cookies["OWNER"]),
        )
        .await;
    assert_eq!(saved.status, 200, "{}", saved.text);
    let two = snapshot(RPC, "OWNER", json!({ "branchId": "B-two" })).await;
    assert_eq!(two.json()["customers"].as_array().map(Vec::len), Some(1));
    let main = snapshot(RPC, "OWNER", json!({})).await;
    // CI-4 (#651): the snapshot lists every LIVE customer of the organisation (owner, 2026-10-09).
    assert_eq!(main.json()["customers"].as_array().map(Vec::len), Some(1));
    // CA-10 #626 (design §6): the owner's direct `adminOpenShift` on the branch they name is
    // retired with the direct shift actions; a shift opens by a count (counts' own tests).
    desk.finish().await;
}

/// A family is marked `WIRED` exactly when its file under
/// `api/src/application/compat/actions/` answers its names — asked of the
/// families the server registers (`registered_actions`), on either route. So a
/// family cannot be marked wired while its names still go through the gap, nor
/// start answering while the tallies still count it as the gap.
#[actix_web::test]
async fn a_family_is_marked_wired_exactly_when_its_file_answers_its_names() {
    let desk = Desk::open().await;
    let actions = registered_actions(&desk.state());
    // one registered file per family, wired or not
    assert_eq!(actions.len(), Family::ALL.len());
    for (family, names, _) in GAP_NAMES {
        for action in names {
            let answered = [RpcRoute::Rpc, RpcRoute::StaffRpc]
                .into_iter()
                .any(|route| actions.iter().any(|a| a.handles(route, action)));
            assert_eq!(
                answered,
                family.wired(),
                "{action}: {family:?} is marked WIRED = {}, but a registered family answering \
                 it is {answered}",
                family.wired()
            );
            assert_eq!(Family::of(action), Some(family), "{action}");
        }
    }
    // every family has its names here, and no part-one name belongs to one
    for family in Family::ALL {
        assert!(GAP_NAMES.iter().any(|(f, _, _)| *f == family), "{family:?}");
    }
    for action in PART_ONE {
        assert_eq!(Family::of(action), None, "{action}");
        assert!(
            !actions
                .iter()
                .any(|a| a.handles(RpcRoute::Rpc, action) || a.handles(RpcRoute::StaffRpc, action)),
            "{action} is the dispatcher's own"
        );
    }
    desk.finish().await;
}
