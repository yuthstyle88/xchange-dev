//! Shared by the policy tests: v2's fixture, its Blueprint re-expression, and the few conversions
//! they need.
//!
//! Provided with ticket T4a; moved by the author of OR-4 (#550, design
//! `docs/design/blueprint/org-roles-branches.md` §4.3) onto `access::Actor` and
//! `Policy::authorize` — the callers are Blueprint users now (§3.3, B1), and a question is still
//! always asked *on a path*. Do not edit, delete or weaken.
//!
//! What the tests call (OR-4's signatures, `[required]`):
//! - `Policy::authorize(actor: Option<&access::Actor>, path: EntryPath, action: &Action,
//!   named: Option<&BranchId>, now: DateTime<Utc>) -> Result<access::Grant, PolicyError>`;
//! - `Policy::need(path: EntryPath, action: &Action) -> Option<Need>`, `Need::{Public,
//!   Permission(Permission), Custody}` in `xc_auth::domain::policy::entities::policy`;
//! - `PolicyError::OutOfScope` (§10 `outOfScope`), the one new sentence.
#![allow(dead_code, unused_imports)]

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

// Every policy path the tests name is in this one block.
pub use xc_auth::domain::access::actor::Actor;
pub use xc_auth::domain::access::authorizer::Grant;
pub use xc_auth::domain::access::permission::Permission;
pub use xc_auth::domain::access::role::Role;
pub use xc_auth::domain::access::role_permissions::RolePermissions;
pub use xc_auth::domain::access::scoped_role::ScopedRole;
pub use xc_auth::domain::policy::entities::policy::{Need, Policy};
pub use xc_auth::domain::policy::value_objects::action::Action;
pub use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
pub use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
pub use xc_kernel::ids::BranchId;

// ── v2's fixture ─────────────────────────────────────────────────────────────

/// What the bridge said behind a redirect (only `GET /staff/desk` has one).
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Behind {
    pub error: String,
    pub status: u16,
}

/// One question put to v2 and what it answered.
#[derive(Deserialize, Clone)]
pub struct Case {
    pub path: String,
    pub route: String,
    /// `ANON`, `OWNER`, or one of the nine staff roles in v2's spelling.
    pub actor: String,
    pub action: String,
    /// The branch a staff caller belongs to; absent for the owner and for nobody.
    pub own: Option<String>,
    /// The branch the request named.
    pub named: Option<String>,
    pub allowed: bool,
    /// The branch v2 went on to work on, where the probe could see it.
    pub branch: Option<String>,
    pub error: Option<String>,
    pub status: Option<u16>,
    pub redirect: Option<String>,
    pub behind: Option<Behind>,
}

#[derive(Deserialize)]
pub struct Refusal {
    pub error: String,
    pub status: u16,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub cases: usize,
    pub allowed: usize,
    pub refused: usize,
    pub redirects: usize,
    pub branch_cases: usize,
    pub paths: usize,
    pub actors: usize,
    pub distinct_refusals: usize,
    pub disagreements: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixture {
    pub counts: Counts,
    pub roles: Vec<String>,
    pub refusals: Vec<Refusal>,
    pub cases: Vec<Case>,
    pub branch_cases: Vec<Case>,
}

pub fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/v2_policy_cases.json")).expect("fixture parses")
}

// ── the Blueprint re-expression (`api/tools/policy-map.mjs`) ────────────────

/// One role row of a mapped caller: `branch` null = the whole organisation.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct RoleRow {
    pub role: String,
    pub branch: Option<String>,
}

/// Case i of `v2_policy_cases.json`, asked by the B1-mapped user.
#[derive(Deserialize, Clone, Debug)]
pub struct Mapped {
    pub path: String,
    pub route: String,
    /// v2's caller, kept to trace the case back.
    pub actor: String,
    /// The mapped caller's roles; null for nobody.
    pub roles: Option<Vec<RoleRow>>,
    pub action: String,
    pub own: Option<String>,
    pub named: Option<String>,
    pub allowed: bool,
    pub branch: Option<String>,
    pub error: Option<String>,
    pub status: Option<u16>,
    pub redirect: Option<String>,
    pub behind: Option<Behind>,
    /// The rule (C1–C8, D4) that changed v2's answer, if any.
    pub change: Option<String>,
    /// `#582` for the custody rows OR-4 does not replay.
    pub owner: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MappedCounts {
    pub cases: usize,
    pub allowed: usize,
    pub refused: usize,
    pub branch_cases: usize,
    pub custody: usize,
    pub changed: usize,
    pub branch_changed: usize,
}

#[derive(Deserialize)]
pub struct Rule {
    pub id: String,
    pub says: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MappedFixture {
    pub counts: MappedCounts,
    pub roles: Vec<String>,
    pub rules: Vec<Rule>,
    pub no_case: Vec<String>,
    pub out_of_scope: Refusal,
    pub refusals: Vec<Refusal>,
    pub cases: Vec<Mapped>,
    pub branch_cases: Vec<Mapped>,
}

pub fn mapped_fixture() -> MappedFixture {
    serde_json::from_str(include_str!("../fixtures/blueprint_policy_cases.json"))
        .expect("blueprint_policy_cases.json parses")
}

// ── names ────────────────────────────────────────────────────────────────────

/// The fixture's name for a path. Exhaustive on purpose: a new path does not compile.
pub fn path_name(path: EntryPath) -> &'static str {
    match path {
        EntryPath::Rpc => "Rpc",
        EntryPath::StaffRpc => "StaffRpc",
        EntryPath::Upload => "Upload",
        EntryPath::StaffUpload => "StaffUpload",
        EntryPath::File => "File",
        EntryPath::StaffFile => "StaffFile",
        EntryPath::StaffDesk => "StaffDesk",
        EntryPath::Devices => "Devices",
        EntryPath::Kiosk => "Kiosk",
        EntryPath::OwnerRoute => "OwnerRoute",
        EntryPath::UserRoute => "UserRoute",
        EntryPath::Custody => "Custody",
        EntryPath::PaymentMethods => "PaymentMethods",
        EntryPath::DrawerPassport => "DrawerPassport",
        EntryPath::DeviceSetup => "DeviceSetup",
        EntryPath::StaffBookings => "StaffBookings",
    }
}

pub const PATHS: [EntryPath; 14] = [
    EntryPath::Rpc,
    EntryPath::StaffRpc,
    EntryPath::Upload,
    EntryPath::StaffUpload,
    EntryPath::File,
    EntryPath::StaffFile,
    EntryPath::StaffDesk,
    EntryPath::Devices,
    EntryPath::Kiosk,
    EntryPath::OwnerRoute,
    EntryPath::UserRoute,
    EntryPath::Custody,
    EntryPath::PaymentMethods,
    EntryPath::DrawerPassport,
];

pub fn path(name: &str) -> EntryPath {
    PATHS
        .into_iter()
        .find(|p| path_name(*p) == name)
        .unwrap_or_else(|| panic!("{name} is not a path of the fixture"))
}

/// The name the code asks with. v2's fixture records the body's action for `OwnerRoute`; the
/// owner routes ask with their route instead (`employees.rs`, `reference_rates.rs`), and that is
/// the row of the route table they hit. (v2's bookings owner route went in BK-11 #1413.)
pub fn asked(route: &str, path: &str, action: &str) -> String {
    match (path, route) {
        ("OwnerRoute", "POST /api/employees") => "employees".to_owned(),
        ("OwnerRoute", "POST /api/reference-rates") => "/api/reference-rates".to_owned(),
        _ => action.to_owned(),
    }
}

pub fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

pub fn action(name: &str) -> Action {
    Action::new(name)
}

// ── people ───────────────────────────────────────────────────────────────────

pub const ORG: &str = "00000000-0000-4000-8000-00000000000a";

/// The moment every question is asked at.
pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 5, 0, 0).unwrap()
}

/// When every role row starts.
pub fn since() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// The nine Blueprint roles in §3.1's order.
pub const BLUEPRINT: [Role; 9] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Teller,
    Role::Finance,
    Role::Marketing,
    Role::Customer,
    Role::VerificationReviewer,
];

pub fn row(role: Role, branch_id: Option<&str>) -> ScopedRole {
    ScopedRole::new(role, branch_id.map(branch), since(), None)
}

pub fn user(id: &str, rows: Vec<ScopedRole>) -> Actor {
    Actor::new(id, ORG, rows, None)
}

/// The owner's user (B3): EXECUTIVE and ADMIN, both organisation-wide.
pub fn owner() -> Actor {
    user(
        "U-OWNER",
        vec![row(Role::Executive, None), row(Role::Admin, None)],
    )
}

/// A user holding one role on one branch (EXECUTIVE and CUSTOMER take none: §3.2).
pub fn staff(r: Role, own: &str) -> Actor {
    let on = match r {
        Role::Executive | Role::Customer => None,
        _ => Some(own),
    };
    user(&format!("U-{}", r.as_str()), vec![row(r, on)])
}

/// B1 (§3.3), written out here and not borrowed from the code under test: v2's caller → the user
/// the mapping makes of them.
pub fn b1(v2: &str) -> &'static [Role] {
    match v2 {
        "OWNER" => &[Role::Executive, Role::Admin],
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("{other} is not a v2 caller"),
    }
}

/// A mapped case's caller, from the roles the fixture gives it.
pub fn caller(rows: Option<&[RoleRow]>) -> Option<Actor> {
    let rows = rows?;
    Some(user(
        "U-MAPPED",
        rows.iter()
            .map(|r| {
                row(
                    Role::parse(&r.role).unwrap_or_else(|| panic!("{} is a role", r.role)),
                    r.branch.as_deref(),
                )
            })
            .collect(),
    ))
}

// ── asking ───────────────────────────────────────────────────────────────────

/// On the caller's own branch (none named).
pub fn ask(who: Option<&Actor>, on: EntryPath, name: &str) -> Result<(), PolicyError> {
    Policy::authorize(who, on, &action(name), None, now()).map(|_| ())
}

/// Naming a branch: the grant says which branch the call runs on.
pub fn ask_on(
    who: Option<&Actor>,
    on: EntryPath,
    name: &str,
    named: Option<&str>,
) -> Result<BranchId, PolicyError> {
    Policy::authorize(who, on, &action(name), named.map(branch).as_ref(), now()).map(|g| g.branch)
}

/// A refusal as the two things a client sees: the sentence and the HTTP status.
pub fn said<T: std::fmt::Debug>(got: Result<T, PolicyError>) -> (String, u16) {
    let e = got.expect_err("must be refused");
    (e.to_string(), e.status())
}

/// What the rule answers, in the shape the fixture records it.
pub fn answer<T>(got: Result<T, PolicyError>) -> Result<(), (String, u16)> {
    got.map(|_| ()).map_err(|e| (e.to_string(), e.status()))
}

/// What a case expects. A redirect carries no sentence of its own: the desk page's is the
/// bridge's answer behind it, and every other one is a route that needs a signed-in caller.
pub fn recorded(
    allowed: bool,
    error: &Option<String>,
    status: Option<u16>,
    behind: &Option<Behind>,
) -> Result<(), (String, u16)> {
    if allowed {
        return Ok(());
    }
    if let Some(b) = behind {
        return Err((b.error.clone(), b.status));
    }
    match (error, status) {
        (Some(e), Some(s)) => Err((e.clone(), s)),
        _ => Err((SIGN_IN.to_owned(), 401)),
    }
}

pub fn recorded_v2(c: &Case) -> Result<(), (String, u16)> {
    recorded(c.allowed, &c.error, c.status, &c.behind)
}

pub fn recorded_mapped(c: &Mapped) -> Result<(), (String, u16)> {
    recorded(c.allowed, &c.error, c.status, &c.behind)
}

pub const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
pub const OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้น";
pub const EMPLOYEE_AUTH: &str = "EMPLOYEE_AUTH_REQUIRED";
pub const INVALID_REQUEST: &str = "คำขอไม่ถูกต้อง";
/// The staff bridge and the custody module.
pub const ROLE_NOT_ALLOWED: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการ";
/// `/api/rpc` for an employee: the same sentence with one more word.
pub const ROLE_NOT_ALLOWED_HERE: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้";
pub const OWNER_ENABLES: &str = "เจ้าของเป็นผู้เปิดใช้ขั้นตอน";
pub const FRONT_ONLY: &str = "ใช้ได้เฉพาะ Front";
pub const DOCUMENTS_NOT_READABLE: &str = "บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้า";
pub const FILES_NOT_READABLE: &str = "บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้าหรือหลักฐานเงิน";
pub const UPLOAD_NOT_ALLOWED: &str = "บทบาทนี้อัปโหลดหลักฐานไม่ได้";
pub const NOT_FOUND: &str = "Not found";
pub const DEVICE_STEP_NOT_ALLOWED: &str = "บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้";
pub const KIOSK_OWNER_OR_RATES: &str = "Owner or Rates access required.";
pub const KIOSK_STAFF: &str = "Staff access required.";
pub const KIOSK_UNKNOWN: &str = "Unknown kiosk action.";
pub const SIGN_NOT_ALLOWED: &str = "ไม่มีสิทธิ์ลงชื่อ";
pub const BACKEND_USES_COMMIT: &str = "Backend ใช้ปุ่มตรวจและบันทึกงาน";
pub const EMPLOYEE_ACCOUNT: &str = "ต้องใช้บัญชีพนักงาน";
pub const CLOCK_IN_FIRST: &str = "ลงเวลาเข้าก่อนทำรายการ";
/// `/api/payment-methods`, `save` by anyone without `SystemSettings` (payment-settings.mjs:48).
pub const PAYMENT_OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้นที่ตั้งค่าวิธีชำระเงินได้";
/// `/api/drawer-passport`, a role outside the desk (passport-drawer.mjs:15).
pub const DRAWER_ROLE_NOT_ALLOWED: &str = "บัญชีนี้ไม่มีสิทธิ์เปิดลิ้นชัก";
/// §10 `outOfScope`: a branch outside the caller's scope (D-4). OR-4's one new sentence.
pub const OUT_OF_SCOPE: &str = "สาขานี้อยู่นอกขอบเขตของบัญชีนี้";
