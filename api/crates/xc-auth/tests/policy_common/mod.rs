//! Shared by the policy tests: the v2 fixture and the few conversions it needs.
//! Provided with ticket T4a — do not edit.
#![allow(dead_code, unused_imports)]

use serde::Deserialize;

// Every policy path the tests name is in this one block.
pub use xc_auth::domain::policy::entities::policy::Policy;
pub use xc_auth::domain::policy::value_objects::action::Action;
pub use xc_auth::domain::policy::value_objects::actor::Actor;
pub use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;
pub use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
pub use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
pub use xc_auth::domain::policy::value_objects::role::Role;
pub use xc_kernel::ids::BranchId;

// ── the fixture ──────────────────────────────────────────────────────────────

/// What the bridge said behind a redirect (only `GET /staff/desk` has one).
#[derive(Deserialize, Clone)]
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

// ── names ────────────────────────────────────────────────────────────────────

/// The fixture's name for a path. Exhaustive on purpose: a thirteenth path does not compile.
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
    }
}

pub const PATHS: [EntryPath; 12] = [
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
];

pub fn path(name: &str) -> EntryPath {
    PATHS
        .into_iter()
        .find(|p| path_name(*p) == name)
        .unwrap_or_else(|| panic!("{name} is not a path of the fixture"))
}

pub fn role(name: &str) -> Role {
    Role::parse(name).unwrap_or_else(|| panic!("{name} is a v2 role"))
}

pub fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

pub fn action(name: &str) -> Action {
    Action::new(name)
}

// ── people ───────────────────────────────────────────────────────────────────

pub fn owner() -> Actor {
    Actor::owner("owner")
}

pub fn staff(r: Role, own: &str) -> Actor {
    Actor::staff(format!("E-{}", r.as_str()), r, branch(own)).expect("a staff role")
}

/// The caller a fixture row names: nobody, the owner, or an employee of `own`.
pub fn caller(actor: &str, own: Option<&str>) -> Option<Actor> {
    match actor {
        "ANON" => None,
        "OWNER" => Some(owner()),
        name => Some(staff(
            role(name),
            own.expect("a staff row carries its branch"),
        )),
    }
}

// ── asking ───────────────────────────────────────────────────────────────────

pub fn ask(who: Option<&Actor>, on: EntryPath, name: &str) -> Result<(), PolicyError> {
    Policy::allow(who, on, &action(name))
}

/// A refusal as the two things a client sees: the sentence and the HTTP status.
pub fn said(got: Result<(), PolicyError>) -> (String, u16) {
    let e = got.expect_err("must be refused");
    (e.to_string(), e.status())
}

/// What the Rust rule answers, in the shape the fixture records it.
pub fn answer(got: Result<(), PolicyError>) -> Result<(), (String, u16)> {
    got.map_err(|e| (e.to_string(), e.status()))
}

/// What v2 answered. A redirect carries no sentence of its own: the desk page's is the
/// bridge's answer behind it, and every other one is a route that needs a signed-in caller
/// (`server.mjs:8278`, which says the sentence below to a POST and redirects a GET).
pub fn recorded(c: &Case) -> Result<(), (String, u16)> {
    if c.allowed {
        return Ok(());
    }
    if let Some(b) = &c.behind {
        return Err((b.error.clone(), b.status));
    }
    match (&c.error, c.status) {
        (Some(e), Some(s)) => Err((e.clone(), s)),
        _ => Err((SIGN_IN.to_owned(), 401)),
    }
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
