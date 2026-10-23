//! Ticket POL (design `docs/design/xchange-v3.md` §5.5.7) — `POST /api/kiosk` asks `Policy`,
//! the kiosk's one role table, who may call which action, for every name a body carries.
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! Each caller v2 knows — nobody, the owner, one employee of each of the nine roles — sends
//! every name of the path `Kiosk` of `crates/xc-auth/tests/fixtures/v2_policy_cases.json`
//! (recorded from the 10-06 bundle by `api/tools/v2-fixtures/gen_policy.mjs`) through the
//! server's whole route table, with a real session made at the real sign-in doors. Where v2
//! refused, the route refuses with v2's status and sentence. Where v2 let the caller through,
//! the answer is not one of the three role sentences — what comes after (a missing id, a
//! closed counter) is not this ticket's.
//!
//! A route that does not ask `Policy` fails here: nothing behind the route asks who may call
//! any more, so nobody signed in would be handed the currencies and the queue.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use compat_common::Person;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const KIOSK: &str = "/api/kiosk";

/// R14, R15, R16 of `PolicyError` — the only sentences a role check says on this path.
const ROLE_SENTENCES: [&str; 3] = [
    "Owner or Rates access required.",
    "Staff access required.",
    "Unknown kiosk action.",
];

/// The front-desk employee whose first password is unchanged.
const MUST_CHANGE: &str = "FRONT_MUST_CHANGE";

/// One row of the path `Kiosk` of the policy fixture.
struct Recorded {
    actor: String,
    action: String,
    /// `None`: let through. `Some((sentence, status))`: refused.
    refusal: Option<(String, u16)>,
}

fn recorded() -> Vec<Recorded> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/crates/xc-auth/tests/fixtures/v2_policy_cases.json"
    );
    let text = std::fs::read_to_string(path).expect("read v2_policy_cases.json");
    let file: Value = serde_json::from_str(&text).expect("v2_policy_cases.json is JSON");
    let rows: Vec<Recorded> = file["cases"]
        .as_array()
        .expect("the fixture's cases")
        .iter()
        .filter(|c| c["path"] == "Kiosk")
        .map(|c| Recorded {
            actor: c["actor"].as_str().expect("actor").to_string(),
            action: c["action"].as_str().expect("action").to_string(),
            refusal: if c["allowed"] == json!(true) {
                None
            } else {
                Some((
                    c["error"]
                        .as_str()
                        .expect("a refusal's sentence")
                        .to_string(),
                    c["status"].as_u64().expect("a refusal's status") as u16,
                ))
            },
        })
        .collect();
    assert_eq!(rows.len(), 11 * 14, "11 callers × the 14 names of the path");
    rows
}

/// What v2 told `actor` for a name it does not answer (the fixture's `nope`).
fn unknown_name_answer(rows: &[Recorded], actor: &str) -> (String, u16) {
    rows.iter()
        .find(|r| r.actor == actor && r.action == "nope")
        .and_then(|r| r.refusal.clone())
        .unwrap_or_else(|| panic!("the fixture refuses `nope` to {actor}"))
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let mut people = one_of_each_role();
        let mut must_change: Person = people
            .iter()
            .find(|p| p.key == "FRONT")
            .expect("a front-desk employee")
            .clone();
        must_change.key = MUST_CHANGE.to_string();
        must_change.id = "00000000-0000-4000-8000-0000000000ff".to_string();
        must_change.username = "mustchange".to_string();
        must_change.must_change = true;
        people.push(must_change);
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room { desk, app, cookies }
    }

    /// `body` sent by `who` (`ANON`: nobody signed in).
    async fn send(&self, who: &str, body: String) -> Answer {
        let call = Call::post(ORIGIN, KIOSK, body);
        let call = match who {
            "ANON" => call,
            key => call.cookie(
                self.cookies
                    .get(key)
                    .unwrap_or_else(|| panic!("{key} is signed in")),
            ),
        };
        self.app.send(call).await
    }

    async fn ask(&self, who: &str, action: &str) -> Answer {
        self.send(who, json!({ "action": action, "payload": {} }).to_string())
            .await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// The sentence of a 400 answer, if it is one of the three role sentences.
fn role_refusal(answer: &Answer) -> Option<String> {
    if answer.status != 400 {
        return None;
    }
    let body: Value = serde_json::from_str(&answer.text).ok()?;
    let sentence = body["error"].as_str()?;
    ROLE_SENTENCES
        .contains(&sentence)
        .then(|| sentence.to_string())
}

fn assert_answered_as_recorded(answer: &Answer, refusal: Option<&(String, u16)>, label: &str) {
    match refusal {
        Some((sentence, status)) => {
            answer.assert_refused(*status, sentence, label);
            assert_eq!(
                answer.header("cache-control"),
                Some("no-store"),
                "{label}: a refusal is never cached"
            );
        }
        None => assert_eq!(
            role_refusal(answer),
            None,
            "{label}: v2 let this caller through; refused by role here: {:.300}",
            answer.text
        ),
    }
}

/// RBAC §11.2 (#3659): the owner holds `QueueOperate` again (via `EXECUTIVE` (`Customers Full`)),
/// and `RATES` (`RATE_MANAGER`) is widened with `QueueOperate` — let through on the queue's work,
/// and answered `Unknown kiosk action.` on a name the kiosk does not know.
fn as_the_blueprint_answers(
    actor: &str,
    action: &str,
    recorded: Option<&(String, u16)>,
) -> Option<(String, u16)> {
    if actor == "RATES" {
        if ["accept", "get", "identityChecked", "publishReceipt"].contains(&action) {
            return None;
        }
        if recorded.is_some_and(|(sentence, _)| sentence == "Staff access required.") {
            return Some(("Unknown kiosk action.".to_string(), 400));
        }
    }
    recorded.cloned()
}

/// D19 (owner rule R-8, Q12 *"เฉพาะพนักงาน (แนะนำ)"*; design `kiosk-shared-phone.md` §4.2 D19,
/// §4.4 B27): only staff issue numbers, so `confirm` is answered as the same caller's `accept`
/// (the R8 rule of `tools/policy-map.mjs`). Every other name keeps v2's answer. Restated by the
/// slice controller (claude/mbp-xc-ks).
fn as_r8_answers(rows: &[Recorded], actor: &str, action: &str) -> Option<(String, u16)> {
    let row = |name: &str| {
        rows.iter()
            .find(|r| r.actor == actor && r.action == name)
            .unwrap_or_else(|| panic!("{actor} has a recorded {name:?}"))
    };
    if action == "confirm" {
        return as_the_blueprint_answers(actor, "accept", row("accept").refusal.as_ref());
    }
    as_the_blueprint_answers(actor, action, row(action).refusal.as_ref())
}

// ── 1. every caller, every name of the recording ─────────────────────────────

#[actix_web::test]
async fn every_caller_gets_v2s_answer_to_every_kiosk_name() {
    let room = Room::open().await;
    let rows = recorded();
    for row in &rows {
        let answer = room.ask(&row.actor, &row.action).await;
        // C8 for the owner's user; R8 for `confirm`
        let expected = as_r8_answers(&rows, &row.actor, &row.action);
        assert_answered_as_recorded(
            &answer,
            expected.as_ref(),
            &format!("{:?} by {}", row.action, row.actor),
        );
    }
    room.finish().await;
}

// ── 2. the 10-06 change: the queue is listed to anybody signed in ────────────

#[actix_web::test]
async fn the_queue_is_listed_to_every_signed_in_role() {
    let room = Room::open().await;
    for person in one_of_each_role() {
        let answer = room.ask(&person.key, "list").await;
        assert_eq!(
            answer.status, 200,
            "list by {}: {:.300}",
            person.key, answer.text
        );
    }
    room.ask("ANON", "list")
        .await
        .assert_refused(400, "Staff access required.", "list by nobody");
    room.finish().await;
}

// ── 3. names v2 does not answer, and bodies that name nothing ────────────────

#[actix_web::test]
async fn a_name_the_kiosk_does_not_answer_is_refused_by_what_the_caller_is() {
    let room = Room::open().await;
    let rows = recorded();
    let names = [
        "",
        "List",
        "list ",
        " list",
        "Manage",
        "listing",
        "adminSnapshot",
        "opsClock",
    ];
    // v2 reads `{ action: name, payload = {} }` from any JSON value but null
    // (`server.mjs:8064` of the 10-06 bundle), so a body naming nothing, or naming it
    // with something that is not text, falls to the end of `action2` like an unknown name
    let bodies = [
        json!({ "payload": {} }).to_string(),
        "{}".to_string(),
        json!({ "action": 5, "payload": {} }).to_string(),
        json!({ "action": null, "payload": {} }).to_string(),
        json!({ "action": ["list"], "payload": {} }).to_string(),
        json!({ "action": { "name": "list" }, "payload": {} }).to_string(),
        "[]".to_string(),
        "5".to_string(),
        "\"list\"".to_string(),
    ];
    let mut callers: Vec<String> = vec!["ANON".to_string()];
    callers.extend(one_of_each_role().into_iter().map(|p| p.key));
    for who in &callers {
        // C8 for the owner's user
        let want = as_the_blueprint_answers(who, "", Some(&unknown_name_answer(&rows, who)))
            .expect("a refusal");
        for name in names {
            let answer = room.ask(who, name).await;
            assert_answered_as_recorded(&answer, Some(&want), &format!("{name:?} by {who}"));
        }
        for body in &bodies {
            let answer = room.send(who, body.clone()).await;
            assert_answered_as_recorded(&answer, Some(&want), &format!("body {body} by {who}"));
        }
    }
    room.finish().await;
}

// ── 4. an employee who must change their password is nobody here ─────────────

#[actix_web::test]
async fn an_employee_who_must_change_their_password_is_answered_as_nobody() {
    let room = Room::open().await;
    let rows = recorded();
    for row in rows.iter().filter(|r| r.actor == "ANON") {
        let answer = room.ask(MUST_CHANGE, &row.action).await;
        // R8: `confirm` is answered as nobody's `accept`
        let expected = if row.action == "confirm" {
            as_r8_answers(&rows, "ANON", "confirm")
        } else {
            row.refusal.clone()
        };
        assert_answered_as_recorded(
            &answer,
            expected.as_ref(),
            &format!(
                "{:?} by a front-desk employee who must change the password",
                row.action
            ),
        );
    }
    room.finish().await;
}
