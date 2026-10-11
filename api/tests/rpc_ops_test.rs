//! Written before the implementation (ticket T11a-C). Do not edit, delete or weaken.
//!
//! The custody actions of the staff route — the thirteen `ops*` names an
//! employee sends to `POST /api/staff/rpc`, and any other `ops…` name — against
//! what v2 recorded in `rpc/ops_actions.json`, **through HTTP**, with the real
//! sessions of ticket T4c.
//!
//! What is replayed: steps #1–#134 of the file, in the order recorded, each one
//! accounted for:
//!
//! - the 95 custody steps that get as far as custody (#1, #2, #22–#134 less the
//!   uploads, the three saves and the two part-one steps): 92 are held to what
//!   v2 answered — status, `Cache-Control`, `Content-Type`, and the body byte
//!   for byte, key order included — and 3 to a decided difference (below);
//! - #3–#8, which `Policy` and the session answer, and #18–#21, #70, #96, the
//!   part-one steps of the day: held to v2 as `rpc_test.rs` already holds them;
//! - #9–#17, the owner's door `/api/employees`: not this ticket's. Custody is
//!   switched on behind the routes where #15 switched it on;
//! - the uploads: stored behind the routes, as `rpc_test.rs` does;
//! - #99–#101, `saveTransaction`: another ticket's (P2-S). While the save family
//!   is not wired the harness holds them to the gap and applies the accepted
//!   save behind the routes, so the drawer is what v2's was at the close.
//!
//! #135–#140 (the views after the day) are not replayed: they show the draft
//! the save of #100 completed, which needs P2-S.
//!
//! Decided differences from v2 (lane controller, ticket T11a-C):
//!
//! - an `ops…` name custody does not know is answered `คำขอไม่ถูกต้อง` whatever
//!   the state of the branch. v2 answers it by that state: #2 (custody off), #23
//!   (not clocked in), #40 (clocked in, nothing found). #62 is the same in both.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use compat_common::{Fixture, Person};
use rpc_common::{
    Answer, App, Call, Cookies, Desk, EMPLOYEE_AUTH_REQUIRED, INVALID_REQUEST, MAIN,
    ROLE_NOT_ALLOWED_RPC, RPC, Replay, Rule, STAFF_RPC, one_of_each_role, sign_in, text_with_ids,
};
use serde_json::{Value, json};
use snapshot_common::time;
use xchange::application::compat::envelope::RpcRoute;
use xchange::presentation::http::rpc::registered_actions;

const ORIGIN: &str = "http://desk.test";
const OPS_FILE: &str = "rpc/ops_actions.json";

/// The thirteen custody actions the staff page and the staff desk send.
const CUSTODY_ACTIONS: [&str; 13] = [
    "opsClock",
    "opsBeginTransaction",
    "opsTarget",
    "opsStartService",
    "opsCutoff",
    "opsPropose",
    "opsHold",
    "opsRelease",
    "opsReject",
    "opsSign",
    "opsDispatch",
    "opsCommit",
    "opsReconcile",
];

/// Custody's own sentences this file names (the rest are read from the file).
const NOT_ENABLED: &str = "เจ้าของต้องเปิดใช้ขั้นตอนรับ–ส่งเงินก่อน";
const OWNER_ENABLES: &str = "เจ้าของเป็นผู้เปิดใช้ขั้นตอน";

/// The last step of the day; the views after it are not this ticket's.
const LAST: usize = 134;
/// `/api/employees`: the owner's door to custody, another ticket's.
const OWNERS_DOOR: std::ops::RangeInclusive<usize> = 9..=17;
/// The step of the owner's door that switched custody on in v2.
const SWITCHED_ON_AT: usize = 15;
/// DECIDED DIFFERENCE (lane controller, T11a-C): an unknown `ops…` name. v2
/// answered these three by the state of the branch; v3 answers
/// `คำขอไม่ถูกต้อง`. They are sent and held to that, not skipped.
const UNKNOWN_NAME_BY_STATE: [usize; 3] = [2, 23, 40];
/// Re-expected for RE-7 #604 by the ticket author: `adminSaveRate`, which RE-7 removes (known
/// difference, design §9 removes the action). The harness holds it to the unknown-name answer
/// (`rpc_common::Rule::Retired`) and publishes the rate v2 accepted, so the later steps keep
/// their meaning; it is counted with the differences.
const RETIRED_SAVE_RATE: [usize; 1] = [70];
/// `saveTransaction`: ticket P2-S. Left to the harness's own rule.
const SAVES: [usize; 3] = [99, 100, 101];
/// The accepted uploads of the day, stored behind the routes.
const UPLOADS: [usize; 14] = [47, 51, 53, 55, 68, 71, 73, 75, 77, 83, 97, 98, 110, 124];
/// The upload v2 refused (a Monitor uploads nothing): not sent, not stored.
const REFUSED_UPLOAD: usize = 50;
/// `opsHold` answers `{saved:true}`: the id of the hold is seen in a snapshot
/// only. The file's next steps release it by that id.
const HOLD_MADE_AT: usize = 105;
const HOLD_NAMED_AT: usize = 106;

fn is_custody_step(fixture: &Fixture, n: usize) -> bool {
    let step = fixture.step(n);
    step.path() == Some(STAFF_RPC)
        && step
            .action
            .as_deref()
            .is_some_and(|action| action.starts_with("ops"))
        // `Policy` answers these two names, and the owner has no staff session
        && !matches!(step.action.as_deref(), Some("opsEnable" | "opsSetEntity"))
}

async fn owner_snapshot(app: &App, cookies: &Cookies, owner: &str, branch: &str) -> Value {
    let answer = app
        .send(
            Call::action(ORIGIN, RPC, "adminSnapshot", json!({ "branchId": branch }))
                .cookie(&cookies[owner]),
        )
        .await;
    assert_eq!(
        answer.status, 200,
        "the owner's snapshot: {:.300}",
        answer.text
    );
    answer.json()
}

fn without_id(rows: &Value) -> Vec<Value> {
    rows.as_array()
        .unwrap_or_else(|| panic!("not a list: {rows}"))
        .iter()
        .map(|row| {
            let mut row = row.clone();
            row.as_object_mut().expect("a row").remove("id");
            row
        })
        .collect()
}

// ── rpc/ops_actions.json ─────────────────────────────────────────────────────

/// The day of `rpc/ops_actions.json`, #1–#134, through HTTP.
#[actix_web::test]
async fn the_custody_day_is_answered_as_v2_answered_it() {
    let fixture = Fixture::load(OPS_FILE);
    assert_eq!(fixture.steps().len(), 140);
    assert_eq!(fixture.step(1).error(), Some(NOT_ENABLED));
    let custody_steps: Vec<usize> = (1..=LAST)
        .filter(|n| is_custody_step(&fixture, *n))
        .collect();
    assert_eq!(custody_steps.len(), 95, "the custody steps of the day");

    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, &fixture).await;
    for n in UNKNOWN_NAME_BY_STATE {
        assert_eq!(
            fixture.step(n).action.as_deref(),
            Some("opsNope"),
            "ops_actions#{n}"
        );
        replay = replay.rule(
            n,
            Rule::Differs {
                status: 400,
                sentence: INVALID_REQUEST,
            },
        );
    }
    for n in OWNERS_DOOR {
        assert_eq!(fixture.step(n).path(), Some("/api/employees"));
        replay = replay.rule(
            n,
            Rule::Excluded("`/api/employees`: the owner's door to custody is another ticket's"),
        );
    }

    let mut same_text = Vec::new();
    for n in 1..=LAST {
        let step = fixture.step(n);
        if n == SWITCHED_ON_AT {
            assert_eq!(step.action.as_deref(), Some("enable"));
            assert_eq!(step.status, Some(200), "v2 switched custody on here");
            replay.desk.custody_on(MAIN).await;
        }
        replay.step(step).await;

        // an accepted custody answer is v2's bytes, key order included
        if custody_steps.contains(&n) && step.status == Some(200) {
            let recorded = fixture.body_text(n).expect("a recorded body");
            assert_eq!(
                replay.answers[&n].text,
                text_with_ids(&replay.ids, step.json(), &recorded),
                "ops_actions#{n}: the body, byte for byte"
            );
            same_text.push(n);
        }
        if n == HOLD_MADE_AT {
            let snapshot = owner_snapshot(&replay.app, &replay.cookies, "owner", MAIN).await;
            let holds = snapshot["operations"]["holds"]
                .as_array()
                .expect("the holds of the branch");
            assert_eq!(holds.len(), 1, "one customer is held after #{n}");
            let recorded = &fixture
                .step(HOLD_NAMED_AT)
                .payload
                .as_ref()
                .expect("a payload")["id"];
            replay.ids.learn(recorded, &holds[0]["id"]);
        }
    }

    // every step of the day, by what was done with it
    let tally = &replay.tally;
    let saves_in_the_gap = if rpc_common::gap_save::WIRED {
        vec![]
    } else {
        SAVES.to_vec()
    };
    let mut differences: Vec<usize> = UNKNOWN_NAME_BY_STATE
        .iter()
        .chain(RETIRED_SAVE_RATE.iter())
        .copied()
        .collect();
    differences.sort_unstable();
    assert_eq!(fixture.step(70).action.as_deref(), Some("adminSaveRate"));
    assert_eq!(tally.differences, differences);
    assert_eq!(tally.part_two, saves_in_the_gap);
    let excluded: Vec<usize> = tally.excluded.iter().map(|(n, _)| *n).collect();
    let mut expected_excluded: Vec<usize> = OWNERS_DOOR.chain(UPLOADS).collect();
    expected_excluded.push(REFUSED_UPLOAD);
    expected_excluded.sort_unstable();
    assert_eq!(excluded, expected_excluded);
    for upload in UPLOADS {
        assert!(tally.applied_behind.contains(&upload), "upload #{upload}");
    }
    assert_eq!(
        tally.total(),
        LAST,
        "every step of the day is accounted for"
    );
    // OR-4b (#767, C3): #19, the owner's adminMoveStock, is refused by role
    assert_eq!(tally.blueprint, vec![19]);
    // 134 − 9 of the owner's door − 15 uploads − 3 saves − 3 differences − 1 refused by role
    // − 1 retired save (RE-7)
    assert_eq!(
        tally.replayed.len(),
        LAST - 9 - 15 - saves_in_the_gap.len() - 3 - 1 - RETIRED_SAVE_RATE.len()
    );
    let held_to_v2: Vec<usize> = custody_steps
        .iter()
        .copied()
        .filter(|n| tally.replayed.contains(n))
        .collect();
    assert_eq!(held_to_v2.len(), 92, "custody steps held to v2");
    assert_eq!(same_text.len(), 39, "accepted custody answers");
    // each of the thirteen was refused and accepted at least once
    for action in CUSTODY_ACTIONS {
        for accepted in [true, false] {
            assert!(
                held_to_v2.iter().any(|n| {
                    let step = fixture.step(*n);
                    step.action.as_deref() == Some(action) && (step.status == Some(200)) == accepted
                }),
                "{action}: no step held to v2 with accepted = {accepted}"
            );
        }
    }

    // what the day left behind is what v2's day left behind (#135, the owner's
    // view): who clocked in and out and when, under whose id and role; the
    // safe; the target; nobody holds the drawer; the requests and how they ended
    let after = owner_snapshot(&replay.app, &replay.cookies, "owner", MAIN).await;
    let ops = &after["operations"];
    let v2 = &fixture.step(135).json()["operations"];
    assert_eq!(
        without_id(&ops["attendance"]),
        without_id(&v2["attendance"]),
        "attendance"
    );
    for key in ["enabled", "phase", "safe", "custodian"] {
        assert_eq!(ops[key], v2[key], "operations.{key}");
    }
    assert_eq!(ops["target"]["amounts"], v2["target"]["amounts"]);
    assert_eq!(ops["target"]["by"], v2["target"]["by"]);
    assert_eq!(ops["target"]["at"], v2["target"]["at"]);
    let ended = |operations: &Value| -> Vec<Value> {
        operations["requests"]
            .as_array()
            .expect("the requests")
            .iter()
            .map(|r| {
                json!([
                    r["requestId"],
                    r["kind"],
                    r["status"],
                    r["at"],
                    r["sender"],
                    r["senderName"],
                    r.get("receiver"),
                    r.get("reconciliation")
                ])
            })
            .collect()
    };
    assert_eq!(ended(ops), ended(v2), "the requests of the day");
    replay.finish().await;
}

// ── which names, on which route ──────────────────────────────────────────────

/// The custody family answers on the staff route only, and never the owner's
/// two names. Asked of the families the server registers, without a request.
#[actix_web::test]
async fn the_custody_family_handles_ops_names_on_the_staff_route_only() {
    let desk = Desk::open().await;
    let families = registered_actions(&desk.state());
    let handlers = |route: RpcRoute, action: &str| {
        families
            .iter()
            .filter(|family| family.handles(route, action))
            .count()
    };
    for action in CUSTODY_ACTIONS
        .iter()
        .chain(&["opsNope", "ops", "opsclock"])
    {
        assert_eq!(
            handlers(RpcRoute::StaffRpc, action),
            1,
            "{action} on the staff route"
        );
        assert_eq!(handlers(RpcRoute::Rpc, action), 0, "{action} on /api/rpc");
    }
    // the owner's two, and names that only look like the family's
    for action in [
        "opsEnable",
        "opsSetEntity",
        "OpsClock",
        "xopsClock",
        "op",
        "",
    ] {
        for route in [RpcRoute::StaffRpc, RpcRoute::Rpc] {
            assert_eq!(handlers(route, action), 0, "{action:?} on {route:?}");
        }
    }
    desk.finish().await;
}

/// The same through HTTP, with custody on and everybody able to clock in:
/// `/api/rpc` knows no custody name for anybody (`rpc/ops_actions.json` #5–#7),
/// the owner has no session on the staff route (#8), and nobody but the owner
/// switches custody on (#3, #4). Nothing of it reaches custody: nobody is
/// clocked in afterwards.
#[actix_web::test]
async fn no_custody_name_is_answered_anywhere_but_the_staff_route() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    desk.custody_on(MAIN).await;

    let clock_in = json!({ "direction": "IN" });
    for action in CUSTODY_ACTIONS.iter().chain(&["opsNope"]) {
        let ask = |path: &'static str, who: &'static str| {
            let call = Call::action(ORIGIN, path, action, clock_in.clone()).cookie(&cookies[who]);
            app.send(call)
        };
        ask(RPC, "OWNER").await.assert_refused(
            400,
            INVALID_REQUEST,
            &format!("{action}: the owner on /api/rpc"),
        );
        ask(RPC, "FRONT").await.assert_refused(
            400,
            ROLE_NOT_ALLOWED_RPC,
            &format!("{action}: a Front on /api/rpc"),
        );
        ask(STAFF_RPC, "OWNER").await.assert_refused(
            401,
            EMPLOYEE_AUTH_REQUIRED,
            &format!("{action}: the owner on the staff route"),
        );
    }
    for action in ["opsEnable", "opsSetEntity"] {
        let call = Call::action(ORIGIN, STAFF_RPC, action, json!({})).cookie(&cookies["FRONT"]);
        app.send(call)
            .await
            .assert_refused(400, OWNER_ENABLES, action);
    }
    // an `ops…` name custody does not know, from somebody not clocked in
    let call = Call::action(ORIGIN, STAFF_RPC, "ops", json!({})).cookie(&cookies["FRONT"]);
    app.send(call)
        .await
        .assert_refused(400, INVALID_REQUEST, "the bare name `ops`");

    let snapshot = owner_snapshot(&app, &cookies, "OWNER", MAIN).await;
    assert_eq!(
        snapshot["operations"]["attendance"],
        json!([]),
        "a refused route clocked somebody in"
    );
    desk.finish().await;
}

// ── who, where and when ──────────────────────────────────────────────────────

fn front_of(branch: &str) -> Person {
    Person {
        key: "ELSEWHERE".to_string(),
        kind: "staff".to_string(),
        id: "00000000-0000-4000-8000-0000000000e1".to_string(),
        username: "elsewhere".to_string(),
        name: "Front Elsewhere".to_string(),
        role: "FRONT".to_string(),
        branch_id: Some(branch.to_string()),
        must_change: false,
    }
}

async fn clock(app: &App, cookies: &Cookies, who: &str, payload: Value) -> Answer {
    app.send(Call::action(ORIGIN, STAFF_RPC, "opsClock", payload).cookie(&cookies[who]))
        .await
}

/// A custody command is run for the caller's own branch, whatever the payload
/// names, at the server's clock, under the caller's account id and role.
#[actix_web::test]
async fn a_custody_command_is_run_for_the_callers_branch_at_the_servers_clock() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    desk.db.seed_branch("B-two", "Two").await;
    let mut people = one_of_each_role();
    people.push(front_of("B-two"));
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;

    // custody is on in MAIN only: an employee of B-two is told theirs is off,
    // also when the payload names MAIN
    desk.custody_on(MAIN).await;
    for payload in [
        json!({ "direction": "IN" }),
        json!({ "direction": "IN", "branchId": MAIN }),
    ] {
        clock(&app, &cookies, "ELSEWHERE", payload)
            .await
            .assert_refused(400, NOT_ENABLED, "custody is off in B-two");
    }

    desk.custody_on("B-two").await;
    desk.clock.set(time("2026-10-06T08:15:30.123Z"));
    let answer = clock(
        &app,
        &cookies,
        "ELSEWHERE",
        json!({ "direction": "IN", "branchId": MAIN }),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.text, "{\"saved\":true}");
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    assert_eq!(answer.header("content-type"), Some("application/json"));

    desk.clock.set(time("2026-10-06T08:20:00.000Z"));
    let rates = clock(&app, &cookies, "RATES", json!({ "direction": "IN" })).await;
    assert_eq!(rates.text, "{\"saved\":true}");

    let two = owner_snapshot(&app, &cookies, "OWNER", "B-two").await;
    assert_eq!(
        without_id(&two["operations"]["attendance"]),
        vec![json!({
            "employeeId": "00000000-0000-4000-8000-0000000000e1",
            "name": "Front Elsewhere",
            "role": "FRONT",
            "in": "2026-10-06T08:15:30.123Z",
            "out": null,
        })],
        "B-two"
    );
    let rates_person = people.iter().find(|p| p.key == "RATES").expect("a RATES");
    let main = owner_snapshot(&app, &cookies, "OWNER", MAIN).await;
    assert_eq!(
        without_id(&main["operations"]["attendance"]),
        vec![json!({
            "employeeId": rates_person.id,
            "name": rates_person.name,
            "role": "RATES",
            "in": "2026-10-06T08:20:00.000Z",
            "out": null,
        })],
        "MAIN"
    );
    desk.finish().await;
}
