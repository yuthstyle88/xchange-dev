//! OR-4b (#550): the routes answer with the Blueprint's permissions (§4.2 C2–C9) and refuse a
//! branch outside the caller's scope (D-4), over HTTP, for callers signed in with today's password
//! sessions (B1 maps them: §3.3).
//!
//! Written before the implementation by the ticket author (design
//! `docs/design/blueprint/org-roles-branches.md` §4.2, §3.5). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! The rule itself is OR-4a's (`api/crates/xc-auth/tests/policy_*_test.rs`); these tests only check
//! that each route asks it. They are written so that they fail on `main` (the routes still ask the
//! v2 rule) and pass once every route asks `Policy::authorize`:
//! - "refused" checks the exact status and sentence;
//! - "let through" checks only that the route's role refusal is not said: what the route does
//!   next with an empty payload is not this ticket's.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";

const ROLE_NOT_ALLOWED: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการ";
const ROLE_NOT_ALLOWED_HERE: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้";
const KIOSK_STAFF: &str = "Staff access required.";
const DEVICE_STEP_NOT_ALLOWED: &str = "บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้";
const OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้น";
const PAYMENT_OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้นที่ตั้งค่าวิธีชำระเงินได้";
/// §10 `outOfScope` (OR-4a's one new sentence).
const OUT_OF_SCOPE: &str = "สาขานี้อยู่นอกขอบเขตของบัญชีนี้";

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room { desk, app, cookies }
    }

    async fn ask(&self, who: &str, path: &str, action: &str, payload: Value) -> Answer {
        let body = json!({ "action": action, "payload": payload }).to_string();
        let cookie = self
            .cookies
            .get(who)
            .unwrap_or_else(|| panic!("{who} is signed in"));
        self.app
            .send(Call::post(ORIGIN, path, body).cookie(cookie))
            .await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn sentence(a: &Answer) -> Option<String> {
    serde_json::from_str::<Value>(&a.text)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_owned))
}

fn refused(a: &Answer, status: u16, want: &str, label: &str) {
    assert_eq!(
        (a.status, sentence(a).as_deref()),
        (status, Some(want)),
        "{label}: {:.300}",
        a.text
    );
}

fn not_refused_with(a: &Answer, not: &str, label: &str) {
    assert_ne!(
        sentence(a).as_deref(),
        Some(not),
        "{label}: still refused by role: {:.300}",
        a.text
    );
}

#[actix_web::test]
async fn the_owners_user_no_longer_sells_or_moves_stock_on_the_owner_rpc() {
    // C2 (§03 B03: ExchangeTransact is TELLER's)
    let room = Room::open().await;
    for action in ["reserveCustomer", "saveTransaction"] {
        let a = room.ask("OWNER", "/api/rpc", action, json!({})).await;
        refused(&a, 400, ROLE_NOT_ALLOWED_HERE, &format!("OWNER {action}"));
    }
    room.finish().await;
}

#[actix_web::test]
async fn the_owners_user_does_not_work_the_kiosk_queue() {
    // C8 (§03 B06: QueueOperate is TELLER's and MANAGER's)
    let room = Room::open().await;
    for action in ["accept", "get", "identityChecked", "publishReceipt"] {
        let a = room.ask("OWNER", "/api/kiosk", action, json!({})).await;
        refused(&a, 400, KIOSK_STAFF, &format!("OWNER kiosk {action}"));
    }
    room.finish().await;
}

#[actix_web::test]
async fn desk_bookings_are_the_tellers() {
    // C9 (§03 B02: BookingsDesk is TELLER's); FRONT (TELLER) keeps them
    let room = Room::open().await;
    for who in ["BACKEND", "MANAGER"] {
        let a = room
            .ask(who, "/api/staff/bookings", "list", json!({}))
            .await;
        refused(
            &a,
            400,
            ROLE_NOT_ALLOWED,
            &format!("{who} staff bookings list"),
        );
    }
    let a = room
        .ask("FRONT", "/api/staff/bookings", "list", json!({}))
        .await;
    not_refused_with(&a, ROLE_NOT_ALLOWED, "FRONT staff bookings list");
    room.finish().await;
}

#[actix_web::test]
async fn ex_managers_print_and_process_device_jobs() {
    // C7: B1 merges BACKEND and MANAGER into BRANCH_MANAGER
    let room = Room::open().await;
    for action in ["printerOnline", "process", "retry", "drawerObservation"] {
        let a = room.ask("MANAGER", "/api/devices", action, json!({})).await;
        not_refused_with(&a, DEVICE_STEP_NOT_ALLOWED, &format!("MANAGER {action}"));
    }
    room.finish().await;
}

#[actix_web::test]
async fn admin_manages_users_and_system_settings() {
    // C4 (§03 A05: UsersManage is ADMIN's), C5 (§02 ADMIN "ตั้งค่าระบบ": SystemSettings);
    // SYSTEM is ADMIN on its branch
    let room = Room::open().await;
    let a = room
        .ask("SYSTEM", "/api/employees", "list", json!({}))
        .await;
    not_refused_with(&a, OWNER_ONLY, "SYSTEM employees list");
    let a = room
        .ask("SYSTEM", "/api/payment-methods", "save", json!({}))
        .await;
    not_refused_with(&a, PAYMENT_OWNER_ONLY, "SYSTEM payment-methods save");
    let a = room
        .ask("SYSTEM", "/api/rpc", "adminCreateBranch", json!({}))
        .await;
    not_refused_with(&a, ROLE_NOT_ALLOWED_HERE, "SYSTEM adminCreateBranch");
    // everybody else but the owner's user is still refused the employees route
    for who in ["FRONT", "BACKEND", "MANAGER", "RATES", "FINANCE", "MONITOR"] {
        let a = room.ask(who, "/api/employees", "list", json!({})).await;
        refused(&a, 403, OWNER_ONLY, &format!("{who} employees list"));
    }
    room.finish().await;
}

#[actix_web::test]
async fn naming_a_branch_outside_scope_is_refused() {
    // D-4 (§02 "Server ตรวจ … branch scope"): the employee of MAIN naming B9 was moved to MAIN; now
    // refused. Naming their own branch still works; the owner's user names any branch.
    let room = Room::open().await;
    for who in ["FRONT", "BACKEND", "RATES"] {
        let a = room
            .ask(who, "/api/devices", "snapshot", json!({ "branchId": "B9" }))
            .await;
        refused(&a, 403, OUT_OF_SCOPE, &format!("{who} devices snapshot B9"));
        let a = room
            .ask(
                who,
                "/api/devices",
                "snapshot",
                json!({ "branchId": "MAIN" }),
            )
            .await;
        assert_ne!(
            sentence(&a).as_deref(),
            Some(OUT_OF_SCOPE),
            "{who} on their own branch: {:.300}",
            a.text
        );
    }
    let a = room
        .ask(
            "OWNER",
            "/api/devices",
            "snapshot",
            json!({ "branchId": "B9" }),
        )
        .await;
    assert_ne!(
        sentence(&a).as_deref(),
        Some(OUT_OF_SCOPE),
        "{:.300}",
        a.text
    );
    room.finish().await;
}
