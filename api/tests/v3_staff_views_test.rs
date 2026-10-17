//! Ticket #573 (S-1) — the four staff views over HTTP: `staff_me`, `staff_floor`, `staff_holds`,
//! `staff_attendance`. Who may read each (by permission, never by role name), what each answers,
//! and the rules the reader writes once: money on the floor only for cash permissions, attendance
//! own rows unless `BranchDayOperate`, the actor's own OPEN shift, hours from `BranchHours`.
//! Written before the implementation by the ticket author (claude/14cc24bd, taking over from
//! claude/89458e53 on the lane controller's order, 2026-10-10). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! Design: `docs/design/xchange-v3-wave3-staff.md` (SD) §2 S2 (no role literal; access is a
//! `Permission`), S4 (paged, newest first), S12 (hours from `BranchHours::window(today)`, none set →
//! absent), §3.1 (the views, their permission and their answers), §3.2 (`StaffFloorReader`), §6.1
//! B3/B5/B9/B10, §8.1 row S-1. Ticket #573 §7–§9 and the lane controller's rulings on #573
//! (2026-10-10): each view is a row of the area file `xc-auth/.../policy/areas/staff_floor.rs` on
//! `EntryPath::StaffRpc`; `staff_holds` admits `CustomerHoldPlace` **or** `VerificationReview`
//! (`Need::AnyOf`, #1149, which #573 waits for).
//!
//! The expectations here are computed from `RolePermissions` and a permission, never from a role
//! name: each case asks "does this actor hold permission P", so the tests hold whatever the role
//! table says. The roles appear only to *make* an actor (through the v2 session, as the staff page
//! signs in today, mapped by `actor_from_v3`).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Each test works in a schema of its own.
//!
//! Signatures and forms this test assumes ([required] in ticket §7 unless marked "choice"):
//! - Four v3 views served by `bootstrap::configure` at `GET /api/v3/views/<name>`, with the seam's
//!   `branch` query key and the seam's answer `{ version, changed, data }`. `staff_holds` and
//!   `staff_attendance` are paged views: `data` is the array of rows, newest first, with the seam's
//!   `before` / `limit`.
//! - Who may read, on the staff session (`EntryPath::StaffRpc`): `staff_me` `SignedIn`;
//!   `staff_floor` and `staff_attendance` `AttendanceClock`; `staff_holds` `CustomerHoldPlace` or
//!   `VerificationReview`. A refusal is the seam's policy answer: no staff session →
//!   `employeeAuthRequired`, a permission not held → `roleNotAllowed`, another branch →
//!   `outOfScope`, each with `PolicyError::status()`.
//! - Answers (camelCase; money as 2-place strings; times as RFC 3339 strings, choice):
//!   - `staff_me`: `userId`, `name` (`users.name`), `roles` `[{ role, branchId }]` (`role` is the
//!     access role's `as_str()`, choice), `branchId`, `branchName` (`branches.name`), `permissions`
//!     (the actor's effective permissions on this branch, each serialised as `Permission` is, so the
//!     page decides with no role literal).
//!   - `staff_floor`: `phase` (`ops_state.phase`), `hours` `{ open, close }` or absent, `myClockIn`
//!     or absent, `myShift` `{ id, openedAt }` or absent, `cutoff` `{ at, pendingCount }` or absent;
//!     and **only** for an actor holding `BranchDayOperate`, `CashTargetSet` or `CashManage`:
//!     `target` `{ amounts: { CUR: "0.00" }, at }` and `safe` `{ CUR: "0.00" }`.
//!   - `staff_holds` rows: `id`, `customerId`, `reason`, `status` (`OPEN` | `RELEASED`), `at`
//!     (`customerName` is optional and not pinned here).
//!   - `staff_attendance` rows: `id`, `name` (`users.name` of the row's person), `in`, `out` or
//!     absent; own rows only, the branch's rows for a holder of `BranchDayOperate`.
//! - "Own" is the person signed in: `attendance.employee_id` and `shifts.teller_id` carry the
//!   signed-in user's id (`Actor::id()` of the staff session = `users.id`).

mod common;
mod lg4_fund;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_auth::domain::access::v3_roles::actor_from_v3;
use xc_auth::domain::access::{Actor as User, Permission, Role, RolePermissions, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor as V2Actor;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_auth::domain::policy::value_objects::role::Role as V2Role;
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_kernel::ids::BranchId;
use xc_rate::domain::branch::branch_hours::{BranchHours, HoursWindow};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const B2: &str = "B2";
/// 2026-10-06 12:00 in Bangkok, in epoch milliseconds (custody times are stored this way).
const T0: i64 = 1_791_262_800_000;

/// Every v2 staff role the staff page can sign in with; each becomes the Blueprint role(s)
/// `actor_from_v3` maps it to. Used only to *make* actors; no expectation names a role.
const V2_STAFF: [V2Role; 9] = [
    V2Role::Front,
    V2Role::Backend,
    V2Role::Manager,
    V2Role::Rates,
    V2Role::Monitor,
    V2Role::Finance,
    V2Role::System,
    V2Role::Documents,
    V2Role::DocumentReview,
];

// ------------------------------------------------------------ the signed-in person

/// Who is signed in, from the header `x-test-as`:
/// - `<V2ROLE>:<user id>:<branch>`: a staff session, mapped as production maps it;
/// - `customer:<user id>:<branch>`: a Blueprint CUSTOMER (holds `SignedIn` only), so a view's
///   refusal of a signed-in person without its permission can be seen;
/// - absent: nobody.
struct HeaderSessions;

fn parse(spec: &str) -> Option<(String, String, String)> {
    let mut p = spec.splitn(3, ':');
    Some((
        p.next()?.to_owned(),
        p.next()?.to_owned(),
        p.next()?.to_owned(),
    ))
}

fn since() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = header(req);
        Box::pin(async move {
            let staff = parse(&spec).and_then(|(role, id, branch)| {
                let role = if role == "customer" {
                    "FRONT".to_owned()
                } else {
                    role
                };
                V2Actor::staff(id, V2Role::parse(&role)?, BranchId::new(branch))
            });
            Sessions { owner: None, staff }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = header(req);
        Box::pin(async move {
            let Some((role, id, branch)) = parse(&spec) else {
                return Callers {
                    owner: None,
                    staff: None,
                    user: None,
                    staff_user: None,
                };
            };
            let s = self.sessions(req).await;
            let staff_user = if role == "customer" {
                Some(User::new(
                    &id,
                    ORG_1,
                    vec![ScopedRole::new(
                        Role::Customer,
                        Some(BranchId::new(branch)),
                        since(),
                        None,
                    )],
                    None,
                ))
            } else {
                s.staff.as_ref().map(|a| actor_from_v3(a, ORG_1, since()))
            };
            Callers {
                owner: None,
                staff: s.staff,
                user: staff_user.clone(),
                staff_user,
            }
        })
    }
}

fn header(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("an ASCII header").to_owned())
        .unwrap_or_default()
}

/// The Blueprint actor the view sees for a staff session of `role` (as production maps it).
fn mapped(role: V2Role, id: &str, branch: &str) -> User {
    let v2 = V2Actor::staff(id, role, BranchId::new(branch)).expect("a staff role");
    actor_from_v3(&v2, ORG_1, since())
}

/// Whether `role`'s session holds `p` on its branch (read from the role table).
fn holds(role: V2Role, p: Permission) -> bool {
    mapped(role, "x", "MAIN")
        .roles
        .iter()
        .any(|r| RolePermissions::of(r.role).contains(&p))
}

/// The actor's effective permissions on its branch, each serialised as `Permission` is.
fn permissions_of(role: V2Role) -> Vec<Value> {
    let mut ps: Vec<Value> = mapped(role, "x", "MAIN")
        .roles
        .iter()
        .flat_map(|r| RolePermissions::of(r.role).iter().copied())
        .map(|p| serde_json::to_value(p).expect("a permission serialises"))
        .collect();
    ps.sort_by_key(|v| v.to_string());
    ps.dedup();
    ps
}

fn spec(role: V2Role, id: &str, branch: &str) -> String {
    format!("{}:{id}:{branch}", role.as_str())
}

// ------------------------------------------------------------ the floor

struct Floor {
    db: TestDb,
}

struct Answer {
    status: StatusCode,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
    fn data(&self) -> &Value {
        &self.body["data"]
    }
}

impl Floor {
    /// MAIN and B2 of the first organisation, with the custody tables empty.
    async fn start() -> Self {
        let db = TestDb::start().await;
        db.branch(B2).await;
        // LG-4 (#929, 1:1): an OPEN count's cash now leaves the branch's ledger SAFE (design §4.4).
        for branch in ["MAIN", B2] {
            lg4_fund::lg4_fund_safe(&db.pool, branch).await;
        }
        Self { db }
    }

    /// A user of the organisation named `name`; answers its id (`users.id`).
    async fn person(&self, name: &str) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG_1)
        .bind(name)
        .fetch_one(&self.db.pool)
        .await
        .expect("insert a user")
    }

    async fn get(&self, who: &str, view: &str, query: &str) -> Answer {
        let state = AppState::new(Arc::new(common::settings(&[])), self.db.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let uri = if query.is_empty() {
            format!("/api/v3/views/{view}")
        } else {
            format!("/api/v3/views/{view}?{query}")
        };
        let mut req = atest::TestRequest::get()
            .uri(&uri)
            .insert_header(("host", HOST))
            .insert_header(("x-xchange-request", "1"));
        if !who.is_empty() {
            req = req.insert_header((AS, who));
        }
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer { status, body }
    }

    async fn ok(&self, who: &str, view: &str, query: &str) -> Value {
        let a = self.get(who, view, query).await;
        assert_eq!(a.status, StatusCode::OK, "{view} as {who}: {}", a.body);
        a.data().clone()
    }

    /// `ops_state` of MAIN.
    async fn ops(
        &self,
        phase: &str,
        target: Option<Value>,
        safe: Option<Value>,
        cutoff: Option<Value>,
    ) {
        sqlx::query(
            "INSERT INTO ops_state (branch_id, phase, target, safe, cutoff) \
             VALUES ('MAIN', $1, $2, $3, $4)",
        )
        .bind(phase)
        .bind(target)
        .bind(safe)
        .bind(cutoff)
        .execute(&self.db.pool)
        .await
        .expect("insert ops_state");
    }

    /// One attendance row of `who` at `branch`, at `position` (higher = newer).
    async fn clock(
        &self,
        branch: &str,
        position: i32,
        who: &str,
        clock_in: i64,
        clock_out: Option<i64>,
    ) -> String {
        let id = unique("att");
        let mut data = json!({ "role": "FRONT", "clockIn": clock_in });
        if let Some(out) = clock_out {
            data["clockOut"] = json!(out);
        }
        sqlx::query(
            "INSERT INTO attendance (id, branch_id, position, employee_id, data) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&id)
        .bind(branch)
        .bind(position)
        .bind(who)
        .bind(data)
        .execute(&self.db.pool)
        .await
        .expect("insert attendance");
        id
    }

    async fn hold(
        &self,
        branch: &str,
        position: i32,
        customer: &str,
        status: &str,
        at: i64,
    ) -> String {
        let id = unique("hold");
        sqlx::query(
            "INSERT INTO customer_holds (id, branch_id, position, data) VALUES ($1, $2, $3, $4)",
        )
        .bind(&id)
        .bind(branch)
        .bind(position)
        .bind(
            json!({ "customerId": customer, "reason": format!("reason {position}"),
                      "by": "someone", "at": at, "status": status }),
        )
        .execute(&self.db.pool)
        .await
        .expect("insert a hold");
        id
    }

    /// MAIN's opening hours: 09:00–20:00 every day, in Asia/Bangkok.
    async fn every_day_nine_to_eight(&self) -> Value {
        let day = json!([{ "open": "09:00", "close": "20:00" }]);
        let hours = json!({ "mon": day, "tue": day, "wed": day, "thu": day, "fri": day,
                            "sat": day, "sun": day });
        sqlx::query(
            "UPDATE branches SET timezone = 'Asia/Bangkok', opening_hours = $1 WHERE id = 'MAIN'",
        )
        .bind(&hours)
        .execute(&self.db.pool)
        .await
        .expect("set MAIN's hours");
        hours
    }

    /// A teller's OPEN shift at MAIN, opened the CA-3 way: the teller counts OPEN, a branch's
    /// manager verifies. Answers the shift id.
    async fn open_shift(&self, teller_id: &str, manager_id: &str) -> String {
        let counts = CashCountService::pg(self.db.pool.clone());
        let teller = mapped(V2Role::Front, teller_id, "MAIN");
        let manager = mapped(V2Role::Manager, manager_id, "MAIN");
        let denomination: i64 = sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'THB' AND face_value = 1000",
        )
        .fetch_one(&self.db.pool)
        .await
        .expect("the THB 1000 face value");
        let count = counts
            .submit(
                &teller,
                SubmitCount {
                    branch: BranchId::new("MAIN"),
                    shift_id: None,
                    kind: CountKind::Open,
                    lines: vec![CountLine::new("THB", denomination, 2)],
                    note: None,
                    idempotency_key: unique("open"),
                },
                Utc::now(),
            )
            .await
            .expect("the teller submits the OPEN count");
        counts
            .verify(&manager, &count.id, Utc::now())
            .await
            .expect("the manager verifies it")
            .shift_id
            .expect("a verified OPEN count names its shift")
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

fn instant(v: &Value, what: &str) -> DateTime<Utc> {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a time string, got {v}"));
    DateTime::parse_from_rfc3339(s)
        .unwrap_or_else(|e| panic!("{what} {s:?}: {e}"))
        .with_timezone(&Utc)
}

fn millis(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .expect("a valid instant")
}

fn money(v: &Value, what: &str) -> Decimal {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a money string, got {v}"));
    assert!(
        s.contains('.') && s.split('.').nth(1).map(str::len) == Some(2),
        "{what}: 2 places, got {s:?}"
    );
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{what} {s:?}: {e}"))
}

fn refused(a: &Answer, err: PolicyError, what: &str) {
    let code = match err {
        PolicyError::EmployeeAuthRequired => "employeeAuthRequired",
        PolicyError::RoleNotAllowed => "roleNotAllowed",
        PolicyError::OutOfScope => "outOfScope",
        other => panic!("not used here: {other:?}"),
    };
    assert_eq!(
        (a.status.as_u16(), a.code()),
        (err.status(), code),
        "{what}: {}",
        a.body
    );
}

const VIEWS: [&str; 4] = ["staff_me", "staff_floor", "staff_holds", "staff_attendance"];

// ------------------------------------------------------------ who may read

#[actix_web::test]
async fn nobody_signed_in_is_asked_to_sign_in_on_every_view() {
    let f = Floor::start().await;
    for view in VIEWS {
        refused(
            &f.get("", view, "").await,
            PolicyError::EmployeeAuthRequired,
            view,
        );
    }
    f.finish().await;
}

#[actix_web::test]
async fn a_signed_in_person_without_the_views_permission_is_refused() {
    let f = Floor::start().await;
    let id = f.person("A Customer").await;
    let who = format!("customer:{id}:MAIN");
    // CUSTOMER holds SignedIn only: staff_me answers, the other three refuse
    assert!(RolePermissions::of(Role::Customer).contains(&Permission::SignedIn));
    for p in [
        Permission::AttendanceClock,
        Permission::CustomerHoldPlace,
        Permission::VerificationReview,
    ] {
        assert!(
            !RolePermissions::of(Role::Customer).contains(&p),
            "premise: {p:?}"
        );
    }
    assert_eq!(f.get(&who, "staff_me", "").await.status, StatusCode::OK);
    for view in ["staff_floor", "staff_holds", "staff_attendance"] {
        refused(
            &f.get(&who, view, "").await,
            PolicyError::RoleNotAllowed,
            view,
        );
    }
    f.finish().await;
}

#[actix_web::test]
async fn each_staff_role_reads_exactly_the_views_its_permissions_allow() {
    let f = Floor::start().await;
    f.ops("READY", None, None, None).await;
    for role in V2_STAFF {
        let id = f.person(&format!("p-{}", role.as_str())).await;
        let who = spec(role, &id, "MAIN");
        let expect = [
            ("staff_me", holds(role, Permission::SignedIn)),
            ("staff_floor", holds(role, Permission::AttendanceClock)),
            (
                "staff_holds",
                holds(role, Permission::CustomerHoldPlace)
                    || holds(role, Permission::VerificationReview),
            ),
            ("staff_attendance", holds(role, Permission::AttendanceClock)),
        ];
        for (view, allowed) in expect {
            let a = f.get(&who, view, "").await;
            if allowed {
                assert_eq!(
                    a.status,
                    StatusCode::OK,
                    "{view} as {}: {}",
                    role.as_str(),
                    a.body
                );
            } else {
                refused(
                    &a,
                    PolicyError::RoleNotAllowed,
                    &format!("{view} as {}", role.as_str()),
                );
            }
        }
    }
    f.finish().await;
}

/// B9: the holds list is for either permission, so a person holding only one of them reads it.
#[actix_web::test]
async fn the_holds_list_is_read_with_either_hold_permission_alone() {
    let f = Floor::start().await;
    let only_place = V2_STAFF
        .into_iter()
        .find(|r| {
            holds(*r, Permission::CustomerHoldPlace) && !holds(*r, Permission::VerificationReview)
        })
        .expect("a role with CustomerHoldPlace and not VerificationReview");
    let only_review = V2_STAFF
        .into_iter()
        .find(|r| {
            holds(*r, Permission::VerificationReview) && !holds(*r, Permission::CustomerHoldPlace)
        })
        .expect("a role with VerificationReview and not CustomerHoldPlace");
    for role in [only_place, only_review] {
        let id = f.person("one of two").await;
        let data = f.ok(&spec(role, &id, "MAIN"), "staff_holds", "").await;
        assert!(data.is_array(), "{}: {data}", role.as_str());
    }
    f.finish().await;
}

#[actix_web::test]
async fn another_branch_is_out_of_scope_on_every_view() {
    let f = Floor::start().await;
    let teller = V2_STAFF
        .into_iter()
        .find(|r| {
            holds(*r, Permission::AttendanceClock) && holds(*r, Permission::CustomerHoldPlace)
        })
        .expect("a role reading all four views");
    let id = f.person("at MAIN").await;
    for view in VIEWS {
        refused(
            &f.get(&spec(teller, &id, "MAIN"), view, &format!("branch={B2}"))
                .await,
            PolicyError::OutOfScope,
            view,
        );
    }
    f.finish().await;
}

// ------------------------------------------------------------ staff_me

#[actix_web::test]
async fn staff_me_answers_the_person_their_roles_branch_and_permissions() {
    let f = Floor::start().await;
    let branch_name =
        f.db.text("SELECT name FROM branches WHERE id = 'MAIN'")
            .await
            .expect("MAIN's name");
    for role in V2_STAFF {
        let id = f.person(&format!("Name {}", role.as_str())).await;
        let me = f.ok(&spec(role, &id, "MAIN"), "staff_me", "").await;
        assert_eq!(me["userId"], json!(id), "{}", role.as_str());
        assert_eq!(me["name"], json!(format!("Name {}", role.as_str())));
        assert_eq!(me["branchId"], json!("MAIN"));
        assert_eq!(me["branchName"], json!(branch_name));
        let roles: Vec<Value> = mapped(role, &id, "MAIN")
            .roles
            .iter()
            .map(|r| json!({ "role": r.role.as_str(), "branchId": "MAIN" }))
            .collect();
        assert_eq!(me["roles"], json!(roles), "{}", role.as_str());
        let mut got: Vec<Value> = me["permissions"].as_array().expect("permissions").clone();
        got.sort_by_key(|v| v.to_string());
        assert_eq!(
            got,
            permissions_of(role),
            "{}: the role table's permissions",
            role.as_str()
        );
    }
    f.finish().await;
}

// ------------------------------------------------------------ staff_floor

#[actix_web::test]
async fn staff_floor_shows_target_and_safe_only_to_cash_permissions() {
    let f = Floor::start().await;
    f.ops(
        "OPEN",
        Some(json!({ "amounts": { "THB": 230000, "USD": 50000 }, "by": "fin", "at": T0 })),
        Some(json!({ "THB": 1_000_000 })),
        None,
    )
    .await;
    for role in V2_STAFF {
        if !holds(role, Permission::AttendanceClock) {
            continue;
        }
        let id = f.person("p").await;
        let floor = f.ok(&spec(role, &id, "MAIN"), "staff_floor", "").await;
        assert_eq!(floor["phase"], json!("OPEN"), "{}", role.as_str());
        let cash = holds(role, Permission::BranchDayOperate)
            || holds(role, Permission::CashTargetSet)
            || holds(role, Permission::CashManage);
        if cash {
            let t = &floor["target"];
            assert_eq!(
                money(&t["amounts"]["THB"], "target THB"),
                Decimal::from(2300)
            );
            assert_eq!(
                money(&t["amounts"]["USD"], "target USD"),
                Decimal::from(500)
            );
            assert_eq!(instant(&t["at"], "target at"), millis(T0));
            assert_eq!(
                money(&floor["safe"]["THB"], "safe THB"),
                Decimal::from(10000)
            );
        } else {
            assert!(
                floor.get("target").is_none_or(Value::is_null)
                    && floor.get("safe").is_none_or(Value::is_null),
                "{}: no cash permission, no money on the floor: {floor}",
                role.as_str()
            );
        }
    }
    f.finish().await;
}

#[actix_web::test]
async fn staff_floor_hours_come_from_the_branchs_own_hours_and_are_absent_when_none_are_set() {
    let f = Floor::start().await;
    f.ops("READY", None, None, None).await;
    let teller = V2_STAFF
        .into_iter()
        .find(|r| holds(*r, Permission::AttendanceClock))
        .unwrap();
    let id = f.person("p").await;
    let who = spec(teller, &id, "MAIN");

    let none = f.ok(&who, "staff_floor", "").await;
    assert!(
        none.get("hours").is_none_or(Value::is_null),
        "S12: none set → absent: {none}"
    );

    let hours = f.every_day_nine_to_eight().await;
    let today = Utc::now().with_timezone(&Tz::Asia__Bangkok).date_naive();
    let HoursWindow::Open { open, close } = BranchHours::new("Asia/Bangkok", Some(&hours))
        .expect("hours parse")
        .window(today)
    else {
        panic!("every day is open");
    };
    let set = f.ok(&who, "staff_floor", "").await;
    assert_eq!(instant(&set["hours"]["open"], "hours.open"), open);
    assert_eq!(instant(&set["hours"]["close"], "hours.close"), close);
    f.finish().await;
}

#[actix_web::test]
async fn staff_floor_my_clock_in_is_my_newest_row_and_only_while_it_is_open() {
    let f = Floor::start().await;
    f.ops("OPEN", None, None, None).await;
    let teller = V2_STAFF
        .into_iter()
        .find(|r| holds(*r, Permission::AttendanceClock))
        .unwrap();
    let me = f.person("me").await;
    let other = f.person("other").await;
    let who = spec(teller, &me, "MAIN");

    // someone else is clocked in; I am not
    f.clock("MAIN", 1, &other, T0, None).await;
    let floor = f.ok(&who, "staff_floor", "").await;
    assert!(
        floor.get("myClockIn").is_none_or(Value::is_null),
        "another's row is not mine: {floor}"
    );

    // an older open row of mine, then a newer closed one: the newest decides
    f.clock("MAIN", 2, &me, T0 + 1_000, None).await;
    f.clock("MAIN", 3, &me, T0 + 2_000, Some(T0 + 3_000)).await;
    let floor = f.ok(&who, "staff_floor", "").await;
    assert!(
        floor.get("myClockIn").is_none_or(Value::is_null),
        "my newest row is closed: {floor}"
    );

    // clocked in again
    f.clock("MAIN", 4, &me, T0 + 4_000, None).await;
    let floor = f.ok(&who, "staff_floor", "").await;
    assert_eq!(
        instant(&floor["myClockIn"], "myClockIn"),
        millis(T0 + 4_000)
    );
    f.finish().await;
}

#[actix_web::test]
async fn staff_floor_my_shift_is_my_own_open_shift_and_not_another_tellers() {
    let f = Floor::start().await;
    f.ops("OPEN", None, None, None).await;
    let a = f.person("teller A").await;
    let b = f.person("teller B").await;
    let manager = f.person("manager").await;
    let a_shift = f.open_shift(&a, &manager).await;
    let opened_at: DateTime<Utc> = sqlx::query_scalar("SELECT opened_at FROM shifts WHERE id = $1")
        .bind(&a_shift)
        .fetch_one(&f.db.pool)
        .await
        .expect("the shift's opened_at");

    let mine = f
        .ok(&spec(V2Role::Front, &a, "MAIN"), "staff_floor", "")
        .await;
    assert_eq!(mine["myShift"]["id"], json!(a_shift));
    assert_eq!(instant(&mine["myShift"]["openedAt"], "openedAt"), opened_at);

    let theirs = f
        .ok(&spec(V2Role::Front, &b, "MAIN"), "staff_floor", "")
        .await;
    assert!(
        theirs.get("myShift").is_none_or(Value::is_null),
        "B has no shift; A's is not B's: {theirs}"
    );
    f.finish().await;
}

#[actix_web::test]
async fn staff_floor_cutoff_answers_its_time_and_how_many_are_pending() {
    let f = Floor::start().await;
    f.ops(
        "CUT_OFF",
        None,
        None,
        Some(json!({ "at": T0, "by": "bm", "shiftId": "s-1", "pending": ["r-1", "r-2"] })),
    )
    .await;
    let teller = V2_STAFF
        .into_iter()
        .find(|r| holds(*r, Permission::AttendanceClock))
        .unwrap();
    let id = f.person("p").await;
    let floor = f.ok(&spec(teller, &id, "MAIN"), "staff_floor", "").await;
    assert_eq!(floor["phase"], json!("CUT_OFF"));
    assert_eq!(instant(&floor["cutoff"]["at"], "cutoff.at"), millis(T0));
    assert_eq!(floor["cutoff"]["pendingCount"], json!(2));
    f.finish().await;
}

// ------------------------------------------------------------ staff_holds

#[actix_web::test]
async fn staff_holds_pages_the_branchs_holds_newest_first() {
    let f = Floor::start().await;
    let h1 = f.hold("MAIN", 1, "c-1", "OPEN", T0).await;
    let h2 = f.hold("MAIN", 2, "c-2", "RELEASED", T0 + 1_000).await;
    let h3 = f.hold("MAIN", 3, "c-3", "OPEN", T0 + 2_000).await;
    f.hold(B2, 1, "c-9", "OPEN", T0).await; // another branch's
    let placer = V2_STAFF
        .into_iter()
        .find(|r| holds(*r, Permission::CustomerHoldPlace))
        .unwrap();
    let id = f.person("p").await;
    let who = spec(placer, &id, "MAIN");

    let all = f.ok(&who, "staff_holds", "").await;
    let ids: Vec<&str> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [h3.as_str(), h2.as_str(), h1.as_str()],
        "S4: newest first, this branch only"
    );
    let first = &all[0];
    assert_eq!(first["customerId"], json!("c-3"));
    assert_eq!(first["reason"], json!("reason 3"));
    assert_eq!(first["status"], json!("OPEN"));
    assert_eq!(instant(&first["at"], "at"), millis(T0 + 2_000));
    assert_eq!(all[1]["status"], json!("RELEASED"));

    let page = f
        .ok(&who, "staff_holds", &format!("limit=1&before={h3}"))
        .await;
    let ids: Vec<&str> = page
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [h2.as_str()], "the page after h3, one row");
    f.finish().await;
}

// ------------------------------------------------------------ staff_attendance

#[actix_web::test]
async fn staff_attendance_is_my_own_rows_unless_i_operate_the_branch_day() {
    let f = Floor::start().await;
    let me = f.person("Me Myself").await;
    let other = f.person("Some Other").await;
    let mine_old = f.clock("MAIN", 1, &me, T0, Some(T0 + 1_000)).await;
    let theirs = f.clock("MAIN", 2, &other, T0 + 2_000, None).await;
    let mine_new = f.clock("MAIN", 3, &me, T0 + 3_000, None).await;
    f.clock(B2, 1, &me, T0, None).await; // another branch's

    for role in V2_STAFF {
        if !holds(role, Permission::AttendanceClock) {
            continue;
        }
        // each role reads as "me" (a new staff session over the same person)
        let rows = f.ok(&spec(role, &me, "MAIN"), "staff_attendance", "").await;
        let ids: Vec<&str> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        if holds(role, Permission::BranchDayOperate) {
            assert_eq!(
                ids,
                [mine_new.as_str(), theirs.as_str(), mine_old.as_str()],
                "{}: BranchDayOperate reads the branch's rows, newest first",
                role.as_str()
            );
        } else {
            assert_eq!(
                ids,
                [mine_new.as_str(), mine_old.as_str()],
                "{}: own rows only, newest first",
                role.as_str()
            );
        }
    }

    let reader = V2_STAFF
        .into_iter()
        .find(|r| holds(*r, Permission::AttendanceClock))
        .unwrap();
    let rows = f
        .ok(&spec(reader, &me, "MAIN"), "staff_attendance", "")
        .await;
    let newest = &rows[0];
    assert_eq!(newest["name"], json!("Me Myself"), "the name by user id");
    assert_eq!(instant(&newest["in"], "in"), millis(T0 + 3_000));
    assert!(
        newest.get("out").is_none_or(Value::is_null),
        "still clocked in"
    );
    let oldest = rows.as_array().unwrap().last().unwrap();
    assert_eq!(instant(&oldest["out"], "out"), millis(T0 + 1_000));

    let page = f
        .ok(
            &spec(reader, &me, "MAIN"),
            "staff_attendance",
            &format!("limit=1&before={mine_new}"),
        )
        .await;
    let ids: Vec<&str> = page
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 1, "one row per the limit");
    f.finish().await;
}
