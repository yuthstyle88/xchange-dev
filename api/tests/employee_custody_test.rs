//! Ticket T11a-E — the owner's door to custody: `/api/employees` `entity`, `enable`, and the
//! cash check of `disable`, answered by the real custody service.
//! Provided with the ticket — do not edit, delete or weaken; add tests in a file of your own.
//!
//! Every test drives the server's own route table (`bootstrap::configure`) with no fake: the
//! custody port is the one the route table registers. They need a real PostgreSQL
//! (`TEST_DATABASE_URL`, a database whose name ends in `_test`); without it they fail.
//!
//! v2's answers are read from `tests/fixtures/rpc/ops_actions.json` (#9–#17),
//! `tests/fixtures/snapshot/custody.json` (#1–#3) and, for `disable`,
//! `tests/fixtures/auth/v2_auth_http_cases.json`, story "employees", steps 31–39 (counted
//! from 1): the only recorded refusals of a `disable`.

mod auth_http_common;
mod common;
mod compat_common;

use std::sync::Arc;

use auth_http_common::*;
use compat_common::Fixture;
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::entities::custody_request::CustodyRequest;
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_event::EventKind;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::custody::value_objects::request_kind::RequestKind;
use xc_shift::domain::custody::value_objects::request_status::RequestStatus;
use xc_shift::domain::custody::value_objects::signature::Signature;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;
use xchange::infrastructure::custody::employee_custody::PgEmployeeCustody;
use xchange::presentation::http::employees::EmployeeCustody;

/// v2, `auth/v2_auth_http_cases.json` story "employees" steps 32, 34, 35.
const HOLDS: &str = "ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน";
/// What the custody rules say to anyone but the owner who sets the entity or switches custody
/// on. The route lets only the owner in (`rpc/ops_actions.json` #12), so v2 never says it here.
const NOT_YOUR_ROLE: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการ";

const SIX: [(&str, &str); 6] = [
    ("fr1", "FRONT"),
    ("be1", "BACKEND"),
    ("mg1", "MANAGER"),
    ("rt1", "RATES"),
    ("mo1", "MONITOR"),
    ("fi1", "FINANCE"),
];

// ── reading and writing custody beside the route ─────────────────────────

fn branch_tx(desk: &Desk) -> BranchTx {
    BranchTx::new(Transactor::new(desk.pool.clone()))
}

/// The custody service on the test's schema, built as the server builds it.
fn custody(desk: &Desk) -> CustodyService {
    CustodyService::new(
        Arc::new(PgCustodyRepository::new(desk.pool.clone())),
        ShiftService::new(
            Arc::new(PgShiftRepository::new(desk.pool.clone())),
            branch_tx(desk),
        ),
        Arc::new(PgCustodyPorts),
        branch_tx(desk),
    )
}

async fn state(desk: &Desk, branch: &str) -> CustodyState {
    custody(desk)
        .state(&BranchId::new(branch))
        .await
        .expect("read the custody state")
}

/// Stores a custody state as the custody service would have left it: the day's work that
/// leads to a custodian or a pending request is #72's and T11a-C's, not this ticket's.
async fn put_state(desk: &Desk, branch: &str, state: CustodyState) {
    let repo = PgCustodyRepository::new(desk.pool.clone());
    let id = BranchId::new(branch);
    let inner = id.clone();
    branch_tx(desk)
        .run(&id, move |tx| {
            Box::pin(async move {
                let before = repo.load_state_in(tx, &inner).await?;
                repo.store_state_in(tx, &inner, &before, &state).await
            })
        })
        .await
        .expect("store the custody state");
}

fn at(millis: i64) -> CustodyTime {
    CustodyTime::from_epoch_millis(1_780_000_000_000 + millis)
}

fn on() -> CustodyState {
    CustodyState {
        enabled: true,
        phase: Phase::Open,
        ..CustodyState::initial()
    }
}

fn request(
    id: &str,
    status: RequestStatus,
    sender: &str,
    receiver: Option<&str>,
) -> CustodyRequest {
    CustodyRequest {
        id: id.to_owned(),
        request_id: format!("page-{id}"),
        payload_hash: format!("hash-{id}"),
        kind: RequestKind::Handover,
        status,
        at: at(0),
        sender: sender.to_owned(),
        receiver: receiver.map(str::to_owned),
        note: String::new(),
        evidence_keys: Vec::new(),
        signatures: Vec::new(),
        shift_id: None,
        counted: None,
        target: None,
        target_difference: None,
        expected: None,
        variance: None,
        cutoff: None,
        safe_count: None,
        safe_expected: None,
        safe_variance: None,
        currency: None,
        amount: None,
        stage: None,
        dispatched_at: None,
        dispatched_by: None,
        rejected: None,
        completed_at: None,
        reconciliation: None,
        reconciled: None,
    }
}

// ── the route ────────────────────────────────────────────────────────────

fn employees(owner: &str, action: &str, payload: Value) -> Call {
    Call::post(
        "/api/employees",
        json!({ "action": action, "payload": payload }),
    )
    .owner(owner)
}

async fn disable(app: &TestApp, owner: &str, id: &str) -> Answer {
    send(app, employees(owner, "disable", json!({ "id": id }))).await
}

async fn create(
    app: &TestApp,
    desk: &Desk,
    owner: &str,
    username: &str,
    role: &str,
    branch: &str,
) -> String {
    let made = send(
        app,
        employees(
            owner,
            "create",
            json!({ "username": username, "name": username, "role": role, "branchId": branch,
                    "password": "first-password-1" }),
        ),
    )
    .await;
    assert_eq!(made.status, 200, "create {username}: {}", made.text);
    desk.employee_id(username).await
}

async fn is_active(desk: &Desk, id: &str) -> bool {
    sqlx::query_scalar("SELECT active FROM employees WHERE id::text = $1")
        .bind(id)
        .fetch_one(&desk.pool)
        .await
        .expect("the employee row")
}

fn refused(answer: &Answer, sentence: &str, why: &str) {
    assert_eq!(
        (answer.status, answer.error().as_str()),
        (400, sentence),
        "{why}"
    );
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"), "{why}");
}

fn saved(answer: &Answer, why: &str) {
    assert_eq!(answer.status, 200, "{why}: {}", answer.text);
    assert_eq!(answer.json(), json!({ "saved": true }), "{why}");
}

/// Sends one recorded step of `/api/employees` and holds the answer to v2's: status, body and
/// the two headers v2 writes.
async fn replay_step(app: &TestApp, fixture: &Fixture, n: usize, owner: &str, staff: &str) {
    let step = fixture.step(n);
    let label = format!("#{n} {}", step.note.as_deref().unwrap_or(""));
    assert_eq!(step.route.as_deref(), Some("/api/employees"), "{label}");
    let call = Call::post(
        "/api/employees",
        json!({ "action": step.action, "payload": step.payload }),
    );
    let call = match step.sent_as.as_str() {
        "owner" => call.owner(owner),
        "FR1" => call.staff(staff),
        other => panic!("{label}: sent by {other}, whom this test does not know"),
    };
    let answer = send(app, call).await;
    assert_eq!(Some(answer.status), step.status, "{label}: {}", answer.text);
    assert_eq!(&answer.json(), step.json(), "{label}");
    let headers = fixture.response_headers(step);
    assert_eq!(
        Some(&answer.content_type),
        headers.get("content-type"),
        "{label}"
    );
    assert_eq!(
        answer.cache_control.as_ref(),
        headers.get("cache-control"),
        "{label}: Cache-Control"
    );
}

// ── entity and enable ────────────────────────────────────────────────────

#[actix_web::test]
async fn ops_actions_9_to_17_are_answered_as_v2_answered() {
    let fixture = Fixture::load("rpc/ops_actions.json");
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let mut front = String::new();
    for (username, role) in SIX {
        let (_, token) = working_employee(&app, &desk, &owner, username, role, "MAIN").await;
        if role == "FRONT" {
            front = token;
        }
    }

    // #9 enable without an entity · #10, #11 an entity that is not complete · #12 not the owner
    for n in 9..=12 {
        replay_step(&app, &fixture, n, &owner, &front).await;
    }
    let before = state(&desk, "MAIN").await;
    assert!(!before.enabled, "nothing accepted yet");
    assert_eq!(before.entity, None, "a refused entity stores nothing");

    // #13 the entity, trimmed
    replay_step(&app, &fixture, 13, &owner, &front).await;
    let named = state(&desk, "MAIN").await;
    let entity = named.entity.clone().expect("#13 stored the entity");
    assert_eq!(
        (
            entity.company.as_str(),
            entity.site.as_str(),
            entity.confirmed_by.as_str()
        ),
        ("XC Exchange Co., Ltd.", "Riverside B", "owner"),
        "trimmed, and confirmed by the actor id"
    );
    assert!(!named.enabled, "an entity does not switch custody on");

    // #14 no branch named (the route's six-role check) · #15 on · #16 on again · #17 locked
    for n in 14..=17 {
        replay_step(&app, &fixture, n, &owner, &front).await;
    }
    let after = state(&desk, "MAIN").await;
    assert!(after.enabled, "#15 switched custody on");
    assert_eq!(
        after.entity,
        Some(entity),
        "#17 was refused: the entity is the one of #13"
    );
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn custody_json_1_and_2_switch_custody_on_through_the_route() {
    let fixture = Fixture::load("snapshot/custody.json");
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    for (username, role) in SIX {
        create(&app, &desk, &owner, username, role, "MAIN").await;
    }
    replay_step(&app, &fixture, 1, &owner, "").await;
    replay_step(&app, &fixture, 2, &owner, "").await;

    // #3 is v2's read of the state the two steps leave ("custody on, nobody clocked in")
    let recorded = &fixture.step(3).json()["operations"];
    let now = state(&desk, "MAIN").await;
    assert_eq!(json!(now.enabled), recorded["enabled"]);
    assert_eq!(recorded["phase"], "CLOSED");
    assert_eq!(now.phase, Phase::Closed);
    assert_eq!(json!(now.custodian), recorded["custodian"]);
    let entity = now.entity.expect("the entity of #1");
    assert_eq!(json!(entity.company), recorded["entity"]["company"]);
    assert_eq!(json!(entity.site), recorded["entity"]["site"]);
    assert_eq!(
        json!(entity.confirmed_by),
        recorded["entity"]["confirmedBy"]
    );

    let events = custody(&desk)
        .events(&BranchId::new("MAIN"))
        .await
        .expect("the custody log");
    let written: Vec<Value> = events
        .iter()
        .map(|e| {
            let kind = match e.kind {
                EventKind::SetEntity => "SET_ENTITY",
                EventKind::Enable => "ENABLE",
                _ => "another kind",
            };
            let role = if e.role == CustodyRole::Owner {
                "OWNER"
            } else {
                "not the owner"
            };
            json!({ "type": kind, "actor": e.actor, "role": role })
        })
        .collect();
    let expected: Vec<Value> = recorded["events"]
        .as_array()
        .expect("recorded events")
        .iter()
        .map(|e| json!({ "type": e["type"], "actor": e["actor"], "role": e["role"] }))
        .collect();
    assert_eq!(written, expected, "the log names the owner by id, as v2's");
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn an_entity_goes_to_the_branch_the_payload_names_and_to_main_when_it_names_none() {
    let desk = Desk::start().await;
    desk.branch("BR-2").await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let entity = |branch: Value, company: &str| {
        let mut payload = json!({ "company": company, "site": "S", "confirmSeparated": true });
        if branch != json!("absent") {
            payload["branchId"] = branch;
        }
        send(&app, employees(&owner, "entity", payload))
    };

    saved(&entity(json!("BR-2"), "Second Co.").await, "BR-2");
    assert_eq!(
        state(&desk, "BR-2").await.entity.map(|e| e.company),
        Some("Second Co.".to_owned())
    );
    assert_eq!(
        state(&desk, "MAIN").await.entity,
        None,
        "MAIN was not named"
    );

    // no `branchId`: MAIN, as `snapshot/custody.json` #1 and `rpc/ops_actions.json` #13
    saved(&entity(json!("absent"), "Main Co.").await, "no branchId");
    assert_eq!(
        state(&desk, "MAIN").await.entity.map(|e| e.company),
        Some("Main Co.".to_owned())
    );
    saved(
        &entity(Value::Null, "Main Again Co.").await,
        "branchId null",
    );
    assert_eq!(
        state(&desk, "MAIN").await.entity.map(|e| e.company),
        Some("Main Again Co.".to_owned())
    );
    assert_eq!(
        state(&desk, "BR-2").await.entity.map(|e| e.company),
        Some("Second Co.".to_owned()),
        "BR-2 keeps its own"
    );
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_branch_id_that_is_not_one_is_a_bad_request() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let long = "a".repeat(71);
    for bad in [
        json!("no such/branch"),
        json!("MAIN "),
        json!(""),
        json!(long),
        json!(7),
        json!(true),
        json!(["MAIN"]),
        json!({ "id": "MAIN" }),
    ] {
        let answer = send(
            &app,
            employees(
                &owner,
                "entity",
                json!({ "branchId": bad, "company": "C", "site": "S", "confirmSeparated": true }),
            ),
        )
        .await;
        refused(&answer, BAD_REQUEST, &format!("entity for branch {bad}"));

        // `enable` through the route never gets this far (no employee is of such a branch):
        // the port itself is asked
        let port = PgEmployeeCustody::new(desk.pool.clone());
        let direct = port
            .enable(&json!({ "branchId": bad }), &Actor::owner("owner"))
            .await;
        assert!(
            matches!(&direct, Err(AppError::Domain(sentence)) if sentence == BAD_REQUEST),
            "enable for branch {bad}: {direct:?}"
        );
    }
    assert_eq!(state(&desk, "MAIN").await, CustodyState::initial());
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM ops_state")
        .fetch_one(&desk.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "nothing was stored for any of them");

    // 70 characters of letters, digits and hyphens is a branch id; this one does not exist
    let unknown = send(
        &app,
        employees(
            &owner,
            "entity",
            json!({ "branchId": "b".repeat(70), "company": "C", "site": "S", "confirmSeparated": true }),
        ),
    )
    .await;
    refused(
        &unknown,
        "ไม่พบสาขาที่เลือก กรุณารีเฟรช",
        "a well-formed id of no branch",
    );
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn the_port_acts_as_the_owner_only_for_the_owner() {
    let desk = Desk::start().await;
    let port = PgEmployeeCustody::new(desk.pool.clone());
    let entity = json!({ "company": "C", "site": "S", "confirmSeparated": true });

    for role in [Role::Front, Role::Manager, Role::Backend, Role::Rates] {
        let employee = Actor::staff(
            "11111111-1111-4111-8111-111111111111",
            role,
            BranchId::new("MAIN"),
        )
        .expect("a staff actor");
        for (name, answer) in [
            ("entity", port.set_entity(&entity, &employee).await),
            ("enable", port.enable(&json!({}), &employee).await),
        ] {
            assert!(
                matches!(&answer, Err(AppError::Domain(sentence)) if sentence == NOT_YOUR_ROLE),
                "{name} by {role:?}: {answer:?}"
            );
        }
    }
    assert_eq!(state(&desk, "MAIN").await, CustodyState::initial());

    let owner = Actor::owner("owner");
    assert_eq!(
        port.set_entity(&entity, &owner)
            .await
            .expect("the owner's entity"),
        json!({ "saved": true })
    );
    assert_eq!(
        port.enable(&json!({ "branchId": "MAIN" }), &owner)
            .await
            .expect("the owner's enable"),
        json!({ "saved": true })
    );
    assert!(state(&desk, "MAIN").await.enabled);
    desk.drop_schema().await;
}

// ── disable: who holds cash or has pending work ──────────────────────────

#[actix_web::test]
async fn the_custodian_cannot_be_disabled() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let holder = create(&app, &desk, &owner, "holder", "FRONT", "MAIN").await;
    let other = create(&app, &desk, &owner, "other", "FRONT", "MAIN").await;
    put_state(
        &desk,
        "MAIN",
        CustodyState {
            custodian: Some(holder.clone()),
            ..on()
        },
    )
    .await;

    // v2: story "employees" steps 31, 32
    refused(
        &disable(&app, &owner, &holder).await,
        HOLDS,
        "the custodian",
    );
    assert!(is_active(&desk, &holder).await, "nobody was disabled");
    // custody is asked about the employee named, not about anyone
    saved(&disable(&app, &owner, &other).await, "not the custodian");
    assert!(!is_active(&desk, &other).await);
    // step 39: an id of nobody is the account action's to answer
    refused(
        &disable(&app, &owner, "nobody").await,
        "ไม่พบพนักงาน",
        "unknown employee",
    );

    // the drawer handed on: he is free to go
    put_state(&desk, "MAIN", on()).await;
    saved(
        &disable(&app, &owner, &holder).await,
        "no longer the custodian",
    );
    assert!(!is_active(&desk, &holder).await);
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn the_sender_and_the_receiver_of_a_pending_request_cannot_be_disabled() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let sender = create(&app, &desk, &owner, "sender", "FRONT", "MAIN").await;
    let receiver = create(&app, &desk, &owner, "receiver", "FRONT", "MAIN").await;
    let bystander = create(&app, &desk, &owner, "bystander", "FINANCE", "MAIN").await;
    put_state(
        &desk,
        "MAIN",
        CustodyState {
            requests: vec![
                request("r0", RequestStatus::Completed, &bystander, None),
                request("r1", RequestStatus::Pending, &sender, Some(&receiver)),
            ],
            ..on()
        },
    )
    .await;

    // v2: story "employees" steps 33, 34, 35
    refused(&disable(&app, &owner, &sender).await, HOLDS, "the sender");
    refused(
        &disable(&app, &owner, &receiver).await,
        HOLDS,
        "the receiver",
    );
    assert!(is_active(&desk, &sender).await && is_active(&desk, &receiver).await);
    saved(
        &disable(&app, &owner, &bystander).await,
        "neither sender nor receiver of the pending one",
    );
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn requests_that_are_no_longer_pending_hold_nobody_back() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let a = create(&app, &desk, &owner, "anan", "FRONT", "MAIN").await;
    let b = create(&app, &desk, &owner, "busaba", "FRONT", "MAIN").await;
    // v2: story "employees" steps 37, 38 — one finished, one rejected, each names both
    put_state(
        &desk,
        "MAIN",
        CustodyState {
            requests: vec![
                request("r1", RequestStatus::Completed, &a, Some(&b)),
                request("r2", RequestStatus::Rejected, &b, Some(&a)),
            ],
            ..on()
        },
    )
    .await;
    saved(&disable(&app, &owner, &a).await, "sender of a finished one");
    saved(&disable(&app, &owner, &b).await, "sender of a rejected one");
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn only_the_custody_of_the_employees_own_branch_is_asked() {
    let desk = Desk::start().await;
    desk.branch("BR-2").await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let away = create(&app, &desk, &owner, "away", "FRONT", "BR-2").await;
    let home = create(&app, &desk, &owner, "home", "FRONT", "MAIN").await;

    // BR-2 says its own employee holds the drawer; MAIN, oddly, names nobody of its own
    put_state(
        &desk,
        "BR-2",
        CustodyState {
            custodian: Some(away.clone()),
            ..on()
        },
    )
    .await;
    refused(&disable(&app, &owner, &away).await, HOLDS, "BR-2 is asked");
    saved(&disable(&app, &owner, &home).await, "MAIN holds nobody");

    // the other way round — v2: story "employees" step 36
    put_state(&desk, "BR-2", on()).await;
    put_state(
        &desk,
        "MAIN",
        CustodyState {
            custodian: Some(away.clone()),
            requests: vec![request("r1", RequestStatus::Pending, &away, Some(&away))],
            ..on()
        },
    )
    .await;
    saved(
        &disable(&app, &owner, &away).await,
        "what MAIN says about an employee of BR-2 does not matter",
    );
    drop(app);
    desk.drop_schema().await;
}

/// No recorded step decides these; the controller's ruling is that none of them holds back.
#[actix_web::test]
async fn being_clocked_in_or_having_signed_another_ones_request_holds_nobody_back() {
    let desk = Desk::start().await;
    let app = desk_app_without_custody!(desk);
    let owner = owner_signs_in(&app).await;
    let sender = create(&app, &desk, &owner, "sender", "FRONT", "MAIN").await;
    let signer = create(&app, &desk, &owner, "signer", "MONITOR", "MAIN").await;
    let mut pending = request("r1", RequestStatus::Pending, &sender, None);
    pending.signatures = vec![Signature {
        employee_id: signer.clone(),
        role: CustodyRole::Monitor,
        at: at(60_000),
    }];
    put_state(
        &desk,
        "MAIN",
        CustodyState {
            custodian: Some(sender.clone()),
            attendance: vec![Attendance {
                id: "att-1".to_owned(),
                employee_id: signer.clone(),
                role: CustodyRole::Monitor,
                clock_in: at(0),
                clock_out: None,
            }],
            requests: vec![pending],
            ..on()
        },
    )
    .await;
    saved(
        &disable(&app, &owner, &signer).await,
        "clocked in, and a signer of a pending request",
    );
    drop(app);
    desk.drop_schema().await;
}
