//! T11e — the envelope of `/api/rpc` and `/api/staff/rpc`.
//!
//! No database, no HTTP server. `rpc/transport.json` holds 94 requests v2
//! answered before any action ran. Each one is accounted for here exactly
//! once:
//!
//! - 30 were answered before the body was read (the same-origin headers, the
//!   session, the method, the path, the host). The route layer decides those;
//!   the envelope only supplies the refusal's shape, which is held to the
//!   recorded status, headers and bytes.
//! - 64 were answered from the body. Those are replayed through
//!   `RpcRequest::parse`, then v2's name check (written out below, because
//!   the check belongs to the dispatcher), then `RpcRequest::payload` — the
//!   order v2 has — and the outcome is held to the recording.
//!
//! Known difference, decided by the lane controller: where v2 answers a
//! malformed body with the JavaScript engine's English, v3 answers
//! `คำขอไม่ถูกต้อง` (steps 18–21, 43, 67–69).

mod compat_common;

use std::collections::BTreeMap;

use compat_common::{Fixture, Step};
use serde_json::{Map, Value, json};
use xchange::application::compat::envelope::{
    MAX_BODY_UTF16_UNITS, RpcRefusal, RpcRequest, RpcResponse, RpcRoute,
};

const TRANSPORT: &str = "rpc/transport.json";

const INVALID: &str = "คำขอไม่ถูกต้อง";
const TOO_LARGE_RPC: &str = "ข้อมูลคำขอใหญ่เกินกำหนด";
const TOO_LARGE_STAFF: &str = "ข้อมูลมากเกินไป";
const ROLE_RPC: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้";
const ROLE_STAFF: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการ";
const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const EMPLOYEE_AUTH: &str = "EMPLOYEE_AUTH_REQUIRED";
const CHANGE_PASSWORD: &str = "กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน";

/// The steps v2 answered in the JavaScript engine's wording.
const ENGINE_WORDING: [usize; 8] = [18, 19, 20, 21, 43, 67, 68, 69];

/// Steps whose body passes the envelope and v2's name check and is refused
/// further in: 44 by the branch rule (`สาขาไม่ถูกต้อง`), 84 by custody.
const REFUSED_FURTHER_IN: [usize; 2] = [44, 84];

fn transport() -> Fixture {
    Fixture::load(TRANSPORT)
}

fn route_of(step: &Step) -> Option<RpcRoute> {
    match step.route.as_deref() {
        Some("/api/rpc") => Some(RpcRoute::Rpc),
        Some("/api/staff/rpc") => Some(RpcRoute::StaffRpc),
        _ => None,
    }
}

/// Why a step was answered before its body was read; `None` when the body
/// decided the answer.
fn answered_before_the_body(fx: &Fixture, step: &Step) -> Option<&'static str> {
    if step.method.as_deref() != Some("POST") {
        return Some("method");
    }
    let Some(route) = route_of(step) else {
        return Some("path or host");
    };
    if step.request_headers.contains_key("origin")
        || step.request_headers.contains_key("x-xchange-request")
    {
        // On `/api/rpc` the session is asked for first (#10), on the staff
        // route the headers are (#58).
        if route == RpcRoute::Rpc && step.sent_as == "nobody" {
            return Some("session");
        }
        return Some("same-origin headers");
    }
    match fx.sender(step) {
        None => Some("session"),
        Some(person) if person.must_change => Some("session"),
        Some(person) if person.is_owner() && route == RpcRoute::StaffRpc => Some("session"),
        Some(_) => None,
    }
}

/// The body a step sent. A long body is written in the file as its length and
/// its beginning: the padding letter is repeated and the two braces closed.
fn body_of(step: &Step) -> String {
    if let Some(raw) = &step.raw {
        return raw.clone();
    }
    let length = step
        .raw_length
        .unwrap_or_else(|| panic!("step {} has no body", step.n));
    let start = step
        .raw_start
        .clone()
        .unwrap_or_else(|| panic!("step {} has no rawStart", step.n));
    let pad = start
        .chars()
        .last()
        .unwrap_or_else(|| panic!("step {}: empty rawStart", step.n));
    assert_eq!(pad.len_utf16(), 1, "step {}", step.n);
    let tail = "\"}}";
    let have = start.encode_utf16().count() + tail.len();
    let mut body = start;
    body.extend(std::iter::repeat_n(pad, length - have));
    body.push_str(tail);
    assert_eq!(body.encode_utf16().count(), length, "step {}", step.n);
    body
}

/// v2's check of the action name, for the callers of this fixture (the owner
/// and a FRONT employee). It is the dispatcher's, not the envelope's; it is
/// written out here so that a replayed step runs the checks in v2's order.
fn v2_name_check(route: RpcRoute, caller_is_staff: bool, action: &str) -> Result<(), RpcRefusal> {
    const READS: [&str; 3] = ["adminSnapshot", "adminDeskConfig", "getConnectionStatus"];
    const FRONT_WRITES: [&str; 2] = ["reserveCustomer", "saveTransaction"];
    const OWNER: [&str; 11] = [
        "adminSnapshot",
        "adminDeskConfig",
        "adminOpenShift",
        "adminCloseShift",
        "adminMoveStock",
        "adminSaveRate",
        "adminCreateBranch",
        "adminAbandonRequest",
        "getConnectionStatus",
        "reserveCustomer",
        "saveTransaction",
    ];
    match (route, caller_is_staff) {
        (RpcRoute::Rpc, false) => {
            if OWNER.contains(&action) {
                Ok(())
            } else {
                Err(RpcRefusal::invalid_request())
            }
        }
        (RpcRoute::Rpc, true) => {
            if READS.contains(&action) || FRONT_WRITES.contains(&action) {
                Ok(())
            } else {
                Err(RpcRefusal::role_not_allowed(route))
            }
        }
        (RpcRoute::StaffRpc, _) => {
            if READS.contains(&action)
                || action == "documents"
                || FRONT_WRITES.contains(&action)
                || action.starts_with("ops")
            {
                Ok(())
            } else {
                Err(RpcRefusal::role_not_allowed(route))
            }
        }
    }
}

/// A body through the envelope and the name check, in v2's order.
fn replay(
    route: RpcRoute,
    caller_is_staff: bool,
    body: &str,
) -> Result<(String, Map<String, Value>), RpcRefusal> {
    let request = RpcRequest::parse(route, body, caller_is_staff)?;
    v2_name_check(route, caller_is_staff, request.action())?;
    request.into_parts()
}

fn headers_of(response: &RpcResponse) -> BTreeMap<String, String> {
    let map: BTreeMap<String, String> = response
        .headers
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    assert_eq!(map.len(), response.headers.len(), "a header twice");
    map
}

/// Holds a refusal's response to what v2 sent for `step`: status, headers and
/// the bytes of the body.
fn assert_answers_as_recorded(fx: &Fixture, step: &Step, refusal: &RpcRefusal) {
    let response = refusal.response();
    assert_eq!(Some(response.status), step.status, "step {}", step.n);
    assert_eq!(
        &headers_of(&response),
        fx.response_headers(step),
        "step {}",
        step.n
    );
    let recorded = fx
        .body_text(step.n)
        .unwrap_or_else(|| panic!("step {} has no JSON body", step.n));
    assert_eq!(response.body, recorded, "step {}", step.n);
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

fn parsed(route: RpcRoute, body: &str, caller_is_staff: bool) -> RpcRequest {
    RpcRequest::parse(route, body, caller_is_staff)
        .unwrap_or_else(|refusal| panic!("{body:?} refused: {refusal:?}"))
}

fn refusal_of(route: RpcRoute, body: &str, caller_is_staff: bool) -> RpcRefusal {
    match RpcRequest::parse(route, body, caller_is_staff) {
        Ok(request) => panic!("{body:?} accepted: {request:?}"),
        Err(refusal) => refusal,
    }
}

fn refused_400(sentence: &str) -> RpcRefusal {
    RpcRefusal {
        status: 400,
        sentence: sentence.to_string(),
        no_store: true,
    }
}

/// A body of exactly `units` UTF-16 units around a padding of `pad`.
fn padded(pad: char, units: usize) -> String {
    let head = "{\"action\":\"getConnectionStatus\",\"payload\":{\"pad\":\"";
    let tail = "\"}}";
    let room = units - head.len() - tail.len();
    let mut body = String::from(head);
    // A letter of two units cannot fill an odd room: one plain letter first.
    body.extend(std::iter::repeat_n('x', room % pad.len_utf16()));
    body.extend(std::iter::repeat_n(pad, room / pad.len_utf16()));
    body.push_str(tail);
    assert_eq!(body.encode_utf16().count(), units);
    body
}

// ---------------------------------------------------------------------------
// The recording, step by step
// ---------------------------------------------------------------------------

#[test]
fn every_step_of_the_recording_is_accounted_for() {
    let fx = transport();
    assert_eq!(fx.steps().len(), 99);
    let mut before = BTreeMap::<&str, Vec<usize>>::new();
    let mut from_body = Vec::new();
    for step in fx.steps() {
        match answered_before_the_body(&fx, step) {
            Some(why) => before.entry(why).or_default().push(step.n),
            None => from_body.push(step.n),
        }
    }
    assert_eq!(before["method"], vec![15, 16, 17, 63, 64, 65]);
    assert_eq!(before["path or host"], vec![92, 93, 94]);
    assert_eq!(
        before["same-origin headers"],
        // 95-99: the same five cases asked by the owner (ticket #146)
        vec![2, 3, 4, 5, 6, 50, 51, 52, 53, 54, 58, 95, 96, 97, 98, 99]
    );
    assert_eq!(
        before["session"],
        vec![9, 10, 11, 12, 14, 57, 59, 60, 61, 62]
    );
    assert_eq!(before.len(), 4);
    assert_eq!(from_body.len(), 64);
    for n in from_body {
        let step = fx.step(n);
        assert!(
            step.raw.is_some() || step.raw_length.is_some(),
            "step {n} was answered from a body the file does not hold"
        );
    }
}

#[test]
fn steps_answered_from_the_body_replay_to_the_recorded_outcome() {
    let fx = transport();
    let mut accepted = 0;
    let mut refused = 0;
    let mut further_in = 0;
    for step in fx.steps() {
        if answered_before_the_body(&fx, step).is_some() {
            continue;
        }
        let n = step.n;
        let route = route_of(step).unwrap_or_else(|| panic!("step {n}: route"));
        let sender = fx
            .sender(step)
            .unwrap_or_else(|| panic!("step {n}: sender"));
        let caller_is_staff = !sender.is_owner();
        let outcome = replay(route, caller_is_staff, &body_of(step));

        if REFUSED_FURTHER_IN.contains(&n) {
            assert_eq!(step.status, Some(400), "step {n}");
            assert!(outcome.is_ok(), "step {n}: {outcome:?}");
            further_in += 1;
            continue;
        }
        match step.status {
            Some(200) => {
                let (action, _) = outcome.unwrap_or_else(|r| panic!("step {n} refused: {r:?}"));
                assert_eq!(action, "getConnectionStatus", "step {n}");
                accepted += 1;
            }
            Some(400) => {
                let refusal = match outcome {
                    Ok(parts) => panic!("step {n} accepted: {parts:?}"),
                    Err(refusal) => refusal,
                };
                if ENGINE_WORDING.contains(&n) {
                    // Known difference: v2's sentence here is the engine's.
                    let recorded = step.error().unwrap_or_else(|| panic!("step {n}: error"));
                    assert!(recorded.is_ascii(), "step {n}: {recorded}");
                    assert_eq!(refusal, refused_400(INVALID), "step {n}");
                    let response = refusal.response();
                    assert_eq!(response.status, 400, "step {n}");
                    assert_eq!(
                        &headers_of(&response),
                        fx.response_headers(step),
                        "step {n}"
                    );
                    assert_eq!(response.body, "{\"error\":\"คำขอไม่ถูกต้อง\"}", "step {n}");
                } else {
                    assert_eq!(Some(refusal.sentence.as_str()), step.error(), "step {n}");
                    assert!(refusal.no_store, "step {n}");
                    assert_answers_as_recorded(&fx, step, &refusal);
                }
                refused += 1;
            }
            other => panic!("step {n}: status {other:?}"),
        }
    }
    assert_eq!((accepted, refused, further_in), (24, 38, 2));
}

/// Which refusals of the recording are the envelope's own (said by `parse` or
/// by `payload`), and which the name check's.
#[test]
fn the_envelope_refuses_exactly_these_steps_itself() {
    let fx = transport();
    let mut by_parse = Vec::new();
    let mut by_payload = Vec::new();
    for step in fx.steps() {
        if answered_before_the_body(&fx, step).is_some() {
            continue;
        }
        let route = route_of(step).unwrap_or_else(|| panic!("step {}: route", step.n));
        let staff = fx.sender(step).is_some_and(|p| !p.is_owner());
        match RpcRequest::parse(route, &body_of(step), staff) {
            Err(_) => by_parse.push(step.n),
            Ok(request) => {
                if request.payload().is_err() {
                    by_payload.push(step.n);
                }
            }
        }
    }
    assert_eq!(
        by_parse,
        vec![
            18, 19, 20, 21, 22, 23, 24, 31, 32, 43, 46, 48, 67, 68, 69, 90
        ]
    );
    assert_eq!(by_payload, vec![26, 27, 28, 29, 42]);
}

#[test]
fn accepted_steps_carry_the_payload_that_was_sent() {
    let fx = transport();
    let cases: [(usize, Value); 14] = [
        (1, json!({})),
        (25, json!({})),
        (30, json!({})),
        (39, json!({"branchId": "no such branch!"})),
        (40, json!({"branchId": "B-nowhere"})),
        (49, json!({})),
        (73, json!({})),
        (74, json!({})),
        (75, json!({})),
        (76, json!({"0": "x"})),
        (77, json!({})),
        (78, json!({})),
        (87, json!({"branchId": "no such branch!"})),
        (88, json!({"branchId": "B-nowhere"})),
    ];
    for (n, payload) in cases {
        let step = fx.step(n);
        let route = route_of(step).unwrap_or_else(|| panic!("step {n}: route"));
        let request = parsed(route, &body_of(step), true);
        assert_eq!(request.route(), route, "step {n}");
        assert_eq!(request.action(), "getConnectionStatus", "step {n}");
        assert_eq!(request.payload(), Ok(&object(payload)), "step {n}");
    }
    for n in [45, 47, 89, 91] {
        let step = fx.step(n);
        let route = route_of(step).unwrap_or_else(|| panic!("step {n}: route"));
        let request = parsed(route, &body_of(step), true);
        let payload = request
            .payload()
            .unwrap_or_else(|r| panic!("step {n}: {r:?}"));
        assert_eq!(payload.len(), 1, "step {n}");
        let pad = payload["pad"]
            .as_str()
            .unwrap_or_else(|| panic!("step {n}: pad"));
        assert!(pad.encode_utf16().count() > 119_900, "step {n}");
    }
}

#[test]
fn a_name_the_role_may_not_use_is_handed_on_not_refused_here() {
    let fx = transport();
    let cases: [(usize, &str); 20] = [
        (33, "nope"),
        (34, "adminsnapshot"),
        (35, "getConnectionStatus "),
        (36, "opsNope"),
        (37, "__proto__"),
        (38, "toString"),
        (41, "nope"),
        (66, ""),
        (70, ""),
        (71, ""),
        (72, ""),
        (79, "7"),
        (80, "getConnectionStatus"),
        (81, "nope"),
        (82, "adminsnapshot"),
        (83, "getConnectionStatus "),
        (84, "opsNope"),
        (85, "__proto__"),
        (86, "toString"),
        (44, "getConnectionStatus"),
    ];
    for (n, action) in cases {
        let step = fx.step(n);
        let route = route_of(step).unwrap_or_else(|| panic!("step {n}: route"));
        let staff = fx.sender(step).is_some_and(|p| !p.is_owner());
        let request = parsed(route, &body_of(step), staff);
        assert_eq!(request.action(), action, "step {n}");
        assert!(request.payload().is_ok(), "step {n}");
    }
}

#[test]
fn steps_answered_before_the_body_have_the_recorded_shape() {
    let fx = transport();
    let mut json_answers = 0;
    let mut plain_answers = 0;
    for step in fx.steps() {
        let Some(why) = answered_before_the_body(&fx, step) else {
            continue;
        };
        let n = step.n;
        if step.text.is_some() {
            // Not an RPC answer at all: the router's plain 404.
            assert_eq!(step.status, Some(404), "step {n}");
            assert_eq!(step.text.as_deref(), Some("Not found"), "step {n}");
            assert!(matches!(why, "method" | "path or host"), "step {n}: {why}");
            plain_answers += 1;
            continue;
        }
        let refusal = match why {
            // ticket #146: v2 tells the owner ORIGIN_REJECTED and an employee คำขอไม่ถูกต้อง
            "same-origin headers" if fx.sender(step).is_some_and(|p| p.is_owner()) => {
                RpcRefusal::refused("ORIGIN_REJECTED")
            }
            "same-origin headers" | "path or host" => RpcRefusal::invalid_request(),
            "session" => {
                let route = route_of(step).unwrap_or_else(|| panic!("step {n}: route"));
                if fx.sender(step).is_some_and(|p| p.must_change) {
                    RpcRefusal::first_password_unchanged(route)
                } else {
                    RpcRefusal::not_signed_in(route)
                }
            }
            other => panic!("step {n}: {other}"),
        };
        assert_eq!(Some(refusal.sentence.as_str()), step.error(), "step {n}");
        assert_answers_as_recorded(&fx, step, &refusal);
        json_answers += 1;
    }
    assert_eq!((json_answers, plain_answers), (27, 8));
}

// ---------------------------------------------------------------------------
// Refusals and their response
// ---------------------------------------------------------------------------

#[test]
fn the_401_of_api_rpc_is_the_only_refusal_without_no_store() {
    let rpc = RpcRefusal::not_signed_in(RpcRoute::Rpc);
    assert_eq!(
        rpc,
        RpcRefusal {
            status: 401,
            sentence: SIGN_IN.to_string(),
            no_store: false
        }
    );
    assert_eq!(
        rpc.response(),
        RpcResponse {
            status: 401,
            headers: vec![("content-type", "application/json")],
            body: "{\"error\":\"กรุณาเข้าสู่ระบบ\"}".to_string(),
        }
    );

    let staff = RpcRefusal::not_signed_in(RpcRoute::StaffRpc);
    assert_eq!(
        staff,
        RpcRefusal {
            status: 401,
            sentence: EMPLOYEE_AUTH.to_string(),
            no_store: true
        }
    );
    assert_eq!(
        staff.response(),
        RpcResponse {
            status: 401,
            headers: vec![
                ("cache-control", "no-store"),
                ("content-type", "application/json")
            ],
            body: "{\"error\":\"EMPLOYEE_AUTH_REQUIRED\"}".to_string(),
        }
    );

    let every_other = [
        RpcRefusal::refused("x"),
        RpcRefusal::invalid_request(),
        RpcRefusal::too_large(RpcRoute::Rpc),
        RpcRefusal::too_large(RpcRoute::StaffRpc),
        RpcRefusal::role_not_allowed(RpcRoute::Rpc),
        RpcRefusal::role_not_allowed(RpcRoute::StaffRpc),
        RpcRefusal::first_password_unchanged(RpcRoute::StaffRpc),
    ];
    for refusal in every_other {
        assert_eq!(refusal.status, 400, "{refusal:?}");
        assert!(refusal.no_store, "{refusal:?}");
        assert_eq!(
            refusal.response().headers,
            vec![
                ("cache-control", "no-store"),
                ("content-type", "application/json")
            ],
            "{refusal:?}"
        );
    }
}

#[test]
fn an_unchanged_first_password_is_said_differently_on_each_route() {
    assert_eq!(
        RpcRefusal::first_password_unchanged(RpcRoute::Rpc),
        RpcRefusal::not_signed_in(RpcRoute::Rpc)
    );
    assert_eq!(
        RpcRefusal::first_password_unchanged(RpcRoute::StaffRpc),
        refused_400(CHANGE_PASSWORD)
    );
}

#[test]
fn the_named_refusals_say_the_recorded_sentences() {
    assert_eq!(RpcRefusal::invalid_request(), refused_400(INVALID));
    assert_eq!(
        RpcRefusal::too_large(RpcRoute::Rpc),
        refused_400(TOO_LARGE_RPC)
    );
    assert_eq!(
        RpcRefusal::too_large(RpcRoute::StaffRpc),
        refused_400(TOO_LARGE_STAFF)
    );
    assert_eq!(
        RpcRefusal::role_not_allowed(RpcRoute::Rpc),
        refused_400(ROLE_RPC)
    );
    assert_eq!(
        RpcRefusal::role_not_allowed(RpcRoute::StaffRpc),
        refused_400(ROLE_STAFF)
    );
    assert_eq!(
        RpcRefusal::refused("ไม่พบสาขาที่เลือก กรุณารีเฟรช"),
        refused_400("ไม่พบสาขาที่เลือก กรุณารีเฟรช")
    );
}

#[test]
fn the_response_body_is_json_whatever_the_sentence_holds() {
    let sentence = "a \"quoted\" \\ line\nbreak – ไทย";
    let response = RpcRefusal::refused(sentence).response();
    let read: Value = serde_json::from_str(&response.body).expect("the body is JSON");
    assert_eq!(read, json!({ "error": sentence }));
    // v2 prints letters as they are, not as escapes.
    assert!(response.body.contains("– ไทย"), "{}", response.body);
    assert!(response.body.contains("a \\\"quoted\\\" \\\\ line\\nbreak"));
    assert!(response.body.starts_with("{\"error\":\""));
}

#[test]
fn a_refusal_of_another_status_keeps_it_in_the_response() {
    let refusal = RpcRefusal {
        status: 500,
        sentence: "x".to_string(),
        no_store: true,
    };
    let response = refusal.response();
    assert_eq!(response.status, 500);
    assert_eq!(response.body, "{\"error\":\"x\"}");
}

// ---------------------------------------------------------------------------
// The size limit
// ---------------------------------------------------------------------------

#[test]
fn the_limit_is_120_000_utf16_units_on_both_routes_for_every_caller() {
    assert_eq!(MAX_BODY_UTF16_UNITS, 120_000);
    let at = padded('x', 120_000);
    let over = padded('x', 120_001);
    for (route, staff, sentence) in [
        (RpcRoute::Rpc, true, TOO_LARGE_RPC),
        (RpcRoute::Rpc, false, TOO_LARGE_RPC),
        (RpcRoute::StaffRpc, true, TOO_LARGE_STAFF),
    ] {
        let request = parsed(route, &at, staff);
        assert_eq!(request.action(), "getConnectionStatus");
        assert_eq!(
            refusal_of(route, &over, staff),
            refused_400(sentence),
            "{route:?} staff={staff}"
        );
    }
}

#[test]
fn letters_are_counted_in_utf16_units_not_bytes_and_not_characters() {
    // Thai: one unit, three bytes each.
    let thai = padded('ก', 120_000);
    assert!(thai.len() > 350_000);
    let thai_over = padded('ก', 120_001);
    // Outside the basic plane: two units, one character each.
    let astral = padded('😀', 120_000);
    assert!(astral.chars().count() < 61_000);
    let astral_over = padded('😀', 120_002);
    assert!(astral_over.chars().count() < 61_000);
    for route in [RpcRoute::Rpc, RpcRoute::StaffRpc] {
        assert!(RpcRequest::parse(route, &thai, true).is_ok(), "{route:?}");
        assert!(RpcRequest::parse(route, &astral, true).is_ok(), "{route:?}");
        assert_eq!(
            refusal_of(route, &thai_over, true),
            RpcRefusal::too_large(route)
        );
        assert_eq!(
            refusal_of(route, &astral_over, true),
            RpcRefusal::too_large(route)
        );
    }
}

#[test]
fn the_size_is_judged_before_the_text_is_read_as_json() {
    let garbage = "x".repeat(120_001);
    assert_eq!(
        refusal_of(RpcRoute::Rpc, &garbage, false),
        refused_400(TOO_LARGE_RPC)
    );
    assert_eq!(
        refusal_of(RpcRoute::StaffRpc, &garbage, true),
        refused_400(TOO_LARGE_STAFF)
    );
    let just_fits = "x".repeat(120_000);
    assert_eq!(
        refusal_of(RpcRoute::Rpc, &just_fits, false),
        refused_400(INVALID)
    );
    assert_eq!(
        refusal_of(RpcRoute::StaffRpc, &just_fits, true),
        refused_400(INVALID)
    );
}

// ---------------------------------------------------------------------------
// `/api/rpc`
// ---------------------------------------------------------------------------

#[test]
fn api_rpc_a_body_that_is_not_json_is_refused_for_every_caller() {
    for body in [
        "",
        " ",
        "not json",
        "{\"action\":\"getConnectionStatus\"",
        "null",
        "{'action':'x'}",
        "{\"action\":\"adminSnapshot\"} trailing",
    ] {
        for staff in [true, false] {
            assert_eq!(
                refusal_of(RpcRoute::Rpc, body, staff),
                refused_400(INVALID),
                "{body:?} staff={staff}"
            );
        }
    }
}

#[test]
fn api_rpc_no_usable_name_is_the_role_sentence_for_staff_and_invalid_for_the_owner() {
    for body in [
        "[]",
        "\"getConnectionStatus\"",
        "{}",
        "7",
        "true",
        "{\"payload\":{}}",
        "{\"action\":null}",
        "{\"action\":7,\"payload\":{}}",
        "{\"action\":true}",
        "{\"action\":{\"name\":\"adminSnapshot\"}}",
        "{\"action\":[\"getConnectionStatus\"],\"payload\":{}}",
        "[{\"action\":\"adminSnapshot\"}]",
    ] {
        assert_eq!(
            refusal_of(RpcRoute::Rpc, body, true),
            refused_400(ROLE_RPC),
            "{body:?}"
        );
        assert_eq!(
            refusal_of(RpcRoute::Rpc, body, false),
            refused_400(INVALID),
            "{body:?}"
        );
    }
}

#[test]
fn api_rpc_the_name_is_judged_before_the_payload() {
    // No usable name and a bad payload: an employee hears the role sentence.
    for body in [
        "{\"action\":7,\"payload\":null}",
        "{\"payload\":[]}",
        "{\"action\":[\"adminSnapshot\"],\"payload\":\"x\"}",
    ] {
        assert_eq!(
            refusal_of(RpcRoute::Rpc, body, true),
            refused_400(ROLE_RPC),
            "{body:?}"
        );
    }
    // A name and a bad payload: the request is handed on with its name, so
    // that the role check can run first; the payload refuses when asked for.
    for staff in [true, false] {
        let request = parsed(
            RpcRoute::Rpc,
            "{\"action\":\"adminSaveRate\",\"payload\":null}",
            staff,
        );
        assert_eq!(request.action(), "adminSaveRate");
        assert_eq!(request.payload(), Err(refused_400(INVALID)));
        assert_eq!(request.into_parts(), Err(refused_400(INVALID)));
    }
}

#[test]
fn api_rpc_the_payload_must_be_an_object_or_absent() {
    for payload in [
        "null", "[]", "[1]", "\"x\"", "\"\"", "7", "0", "true", "false",
    ] {
        let body = format!("{{\"action\":\"adminSnapshot\",\"payload\":{payload}}}");
        for staff in [true, false] {
            let request = parsed(RpcRoute::Rpc, &body, staff);
            assert_eq!(request.action(), "adminSnapshot", "{body}");
            assert_eq!(request.payload(), Err(refused_400(INVALID)), "{body}");
        }
    }
    let absent = parsed(RpcRoute::Rpc, "{\"action\":\"adminSnapshot\"}", false);
    assert_eq!(absent.payload(), Ok(&Map::new()));
    let whole = parsed(
        RpcRoute::Rpc,
        "{\"action\":\"adminSaveRate\",\"payload\":{\"currency\":\"USD\",\"buy\":32.5,\"nested\":{\"a\":[1,null]}},\"more\":1}",
        false,
    );
    assert_eq!(
        whole.into_parts(),
        Ok((
            "adminSaveRate".to_string(),
            object(json!({"currency": "USD", "buy": 32.5, "nested": {"a": [1, null]}}))
        ))
    );
}

#[test]
fn api_rpc_a_name_is_taken_as_written() {
    for name in ["", " adminSnapshot", "ADMINSNAPSHOT", "ops", "ชื่อ", "7"] {
        let body = json!({"action": name, "payload": {}}).to_string();
        for staff in [true, false] {
            assert_eq!(parsed(RpcRoute::Rpc, &body, staff).action(), name);
        }
    }
}

#[test]
fn a_key_written_twice_counts_once_the_last_one() {
    for route in [RpcRoute::Rpc, RpcRoute::StaffRpc] {
        let request = parsed(
            route,
            "{\"action\":\"a\",\"payload\":{\"k\":1},\"action\":\"b\",\"payload\":{\"k\":2}}",
            true,
        );
        assert_eq!(request.action(), "b");
        assert_eq!(request.payload(), Ok(&object(json!({"k": 2}))));
    }
}

// ---------------------------------------------------------------------------
// `/api/staff/rpc`
// ---------------------------------------------------------------------------

#[test]
fn staff_route_an_empty_body_is_read_as_an_empty_object() {
    let request = parsed(RpcRoute::StaffRpc, "", true);
    assert_eq!(request.action(), "");
    assert_eq!(request.payload(), Ok(&Map::new()));
    // Only a body of no letters at all: blanks are not JSON.
    for body in [" ", "\n", "not json", "{\"action\":\"x\"", "null"] {
        assert_eq!(
            refusal_of(RpcRoute::StaffRpc, body, true),
            refused_400(INVALID),
            "{body:?}"
        );
    }
}

#[test]
fn staff_route_a_body_that_is_not_an_object_has_no_name_and_no_payload() {
    for body in [
        "[]",
        "[{\"action\":\"adminSnapshot\"}]",
        "\"adminSnapshot\"",
        "7",
        "0",
        "true",
        "false",
        "{}",
    ] {
        let request = parsed(RpcRoute::StaffRpc, body, true);
        assert_eq!(request.action(), "", "{body:?}");
        assert_eq!(request.payload(), Ok(&Map::new()), "{body:?}");
    }
}

#[test]
fn staff_route_the_name_is_made_into_text_as_v2_makes_it() {
    let cases: [(&str, &str); 16] = [
        ("\"adminSnapshot\"", "adminSnapshot"),
        ("[\"getConnectionStatus\"]", "getConnectionStatus"),
        ("[[\"getConnectionStatus\"]]", "getConnectionStatus"),
        ("[\"a\",\"b\"]", "a,b"),
        ("[\"opsPropose\",null]", "opsPropose,"),
        ("[null]", ""),
        ("[]", ""),
        ("[1,true,[\"x\",2]]", "1,true,x,2"),
        ("[{}]", "[object Object]"),
        ("{\"name\":\"adminSnapshot\"}", "[object Object]"),
        ("7", "7"),
        ("true", "true"),
        ("null", ""),
        ("false", ""),
        ("0", ""),
        ("\"\"", ""),
    ];
    for (written, name) in cases {
        let body = format!("{{\"action\":{written},\"payload\":{{}}}}");
        let request = parsed(RpcRoute::StaffRpc, &body, true);
        assert_eq!(request.action(), name, "{body}");
    }
}

#[test]
fn staff_route_any_payload_is_accepted_and_made_an_object() {
    let cases: [(&str, Value); 15] = [
        ("null", json!({})),
        ("false", json!({})),
        ("0", json!({})),
        ("0.0", json!({})),
        ("-0", json!({})),
        ("\"\"", json!({})),
        ("7", json!({})),
        ("true", json!({})),
        ("[]", json!({})),
        ("{}", json!({})),
        ("\"x\"", json!({"0": "x"})),
        ("\"กข\"", json!({"0": "ก", "1": "ข"})),
        (
            "[\"a\",{\"b\":1},null]",
            json!({"0": "a", "1": {"b": 1}, "2": null}),
        ),
        ("{\"currency\":\"USD\"}", json!({"currency": "USD"})),
        (
            "{\"branchId\":\"B-2\",\"n\":[1]}",
            json!({"branchId": "B-2", "n": [1]}),
        ),
    ];
    for (written, payload) in cases {
        let body = format!("{{\"action\":\"adminSnapshot\",\"payload\":{written}}}");
        let request = parsed(RpcRoute::StaffRpc, &body, true);
        assert_eq!(request.action(), "adminSnapshot", "{body}");
        assert_eq!(request.payload(), Ok(&object(payload.clone())), "{body}");
        assert_eq!(
            request.into_parts(),
            Ok(("adminSnapshot".to_string(), object(payload))),
            "{body}"
        );
    }
    let absent = parsed(RpcRoute::StaffRpc, "{\"action\":\"adminSnapshot\"}", true);
    assert_eq!(absent.payload(), Ok(&Map::new()));
}

#[test]
fn staff_route_does_not_ask_who_the_caller_is() {
    for body in [
        "",
        "{}",
        "{\"action\":7}",
        "{\"action\":\"adminSnapshot\",\"payload\":null}",
        "{\"action\":[\"adminSnapshot\"],\"payload\":[1]}",
        "null",
        "not json",
    ] {
        assert_eq!(
            RpcRequest::parse(RpcRoute::StaffRpc, body, true),
            RpcRequest::parse(RpcRoute::StaffRpc, body, false),
            "{body:?}"
        );
    }
}

#[test]
fn the_two_routes_read_the_same_body_differently() {
    // One name in an array: a name on the staff route, none on `/api/rpc`.
    let body = "{\"action\":[\"getConnectionStatus\"],\"payload\":{}}";
    assert_eq!(
        parsed(RpcRoute::StaffRpc, body, true).action(),
        "getConnectionStatus"
    );
    assert_eq!(refusal_of(RpcRoute::Rpc, body, true), refused_400(ROLE_RPC));
    // A payload that is not an object.
    let body = "{\"action\":\"getConnectionStatus\",\"payload\":[7]}";
    assert_eq!(
        parsed(RpcRoute::StaffRpc, body, true).payload(),
        Ok(&object(json!({"0": 7})))
    );
    assert_eq!(
        parsed(RpcRoute::Rpc, body, true).payload(),
        Err(refused_400(INVALID))
    );
    // No body at all.
    assert!(RpcRequest::parse(RpcRoute::StaffRpc, "", true).is_ok());
    assert!(RpcRequest::parse(RpcRoute::Rpc, "", true).is_err());
}

#[test]
fn staff_route_a_name_written_as_minus_zero_is_the_empty_name() {
    // v2's `String(body.action || "")`: `-0 || ""` is `""`, as `0 || ""` is.
    // `serde_json` reads `-0` as the double -0.0, which is not `0.0` by sign.
    for written in ["-0", "-0.0", "-0e3"] {
        let body = format!("{{\"action\":{written},\"payload\":{{}}}}");
        let request = parsed(RpcRoute::StaffRpc, &body, true);
        assert_eq!(request.action(), "", "{body}");
    }
}

#[test]
fn staff_route_a_text_payload_is_spread_per_utf16_unit_not_per_letter() {
    // v2's `{ ..."😀" }` has the keys `0` and `1`, one half of the surrogate
    // pair each. Rust text cannot hold half a pair, so each half is U+FFFD
    // here; what must hold is the count and the keys, one per UTF-16 unit.
    let half = "\u{FFFD}";
    let cases: [(&str, Value); 4] = [
        ("\"😀\"", json!({"0": half, "1": half})),
        ("\"\\ud83d\\ude00\"", json!({"0": half, "1": half})),
        (
            "\"a😀b\"",
            json!({"0": "a", "1": half, "2": half, "3": "b"}),
        ),
        (
            "\"ก😀😀\"",
            json!({"0": "ก", "1": half, "2": half, "3": half, "4": half}),
        ),
    ];
    for (written, payload) in cases {
        let body = format!("{{\"action\":\"adminSnapshot\",\"payload\":{written}}}");
        let request = parsed(RpcRoute::StaffRpc, &body, true);
        assert_eq!(request.payload(), Ok(&object(payload)), "{body}");
    }
}
