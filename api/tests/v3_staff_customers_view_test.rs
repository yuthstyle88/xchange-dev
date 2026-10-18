//! Ticket #580, its S-2 part: the staff page's customer search, the view `staff_customers`.
//! Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane
//! controller claude/5edf7bb5's order; the owner ordered it on 2026-10-11). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! Design: `docs/design/xchange-v3-wave3-staff.md` (SD) §3.1 row `staff_customers`: `REQUIRES: CustomerHoldPlace`;
//! paged; filter `q` required, an empty `q` gives an empty page; answers `[{id, displayName}]`; reads
//! `xc-customer`'s search: a name, a customer id or a passport number. §2 S11: nothing is listed until a key is
//! typed; a passport number is matched through `xc-customer`'s lookup hash (#554 K4). §8.1 row S-2: "empty `q` →
//! empty page; a name, a customer id and a passport number (any spacing/case) each find the customer; the answer
//! never carries an identity value". #554 K8: no list carries an identity value. Ticket #580 §7 and §9.
//!
//! What this test assumes ([required] unless marked "choice"):
//! - The view `staff_customers`, served by `bootstrap::configure` at `GET /api/v3/views/staff_customers` with the
//!   seam's answer `{ version, changed, data }`, `data` the array of rows (a paged view, as S-1's
//!   `staff_holds`).
//! - Who may read: the staff session (`EntryPath::StaffRpc`, as S-1's four views), the row
//!   `(StaffRpc, "staffCustomers") → Need::Permission(CustomerHoldPlace)` (choice of name, after S-1's `staffMe`,
//!   `staffFloor`, `staffHolds`, `staffAttendance`; its area file is `xc-auth/.../policy/areas/staff_customers.rs`,
//!   added to #580's §5 by the FE lane controller, #580 issuecomment-6107999524). Refusals are the seam's: no staff session →
//!   `employeeAuthRequired`; a permission not held → `roleNotAllowed`; each with `PolicyError::status()`.
//! - A row is exactly `{ id, displayName }`: `customers.id` and `customers.display_name`. Nothing else.
//! - Matching: the name and the customer id case-insensitively, after trimming the key; a passport number with
//!   any spacing, dashes or case (the normaliser's rules, `xc-kernel/src/pii/normalize.rs`). The lookup hash
//!   includes the nationality, so the server hashes the typed number under each nationality on file (choice of the
//!   implementer; only the behaviour is pinned).
//!
//! Customers are made through `CustomerService::reserve`, as the desk makes them (one customer per organisation,
//! the passport encrypted with its lookup hash). Expectations about who may read are computed from
//! `RolePermissions`, never from a role name; roles only *make* an actor. Needs `TEST_DATABASE_URL`; nothing
//! skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use pos_infra::transactions::Transactor;
use quote_common::*;
use serde_json::Value;
use xc_auth::domain::access::v3_roles::actor_from_v3;
use xc_auth::domain::access::{Actor as User, Permission, Role, RolePermissions, ScopedRole};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor as V2Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_auth::domain::policy::value_objects::role::Role as V2Role;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const VIEW: &str = "staff_customers";

/// Every v2 staff role the staff page signs in with, mapped to Blueprint roles as production maps them.
/// Used only to *make* actors; no expectation names a role.
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

// ------------------------------------------------------------ the signed-in person (as S-1's test)

/// `<V2ROLE>:<user id>:<branch>`: a staff session; `customer:<user id>:<branch>`: a signed-in Blueprint CUSTOMER
/// (`SignedIn` only); absent: nobody.
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

fn header(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("an ASCII header").to_owned())
        .unwrap_or_default()
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
                user: staff_user.clone(),
                staff_user,
            }
        })
    }
}

fn mapped(role: V2Role) -> User {
    let v2 = V2Actor::staff("x", role, BranchId::new("MAIN")).expect("a staff role");
    actor_from_v3(&v2, ORG_1, since())
}

/// Whether `role`'s session holds `p` on its branch (read from the role table).
fn holds(role: V2Role, p: Permission) -> bool {
    mapped(role)
        .roles
        .iter()
        .any(|r| RolePermissions::of(r.role).contains(&p))
}

/// A staff role whose session holds `CustomerHoldPlace`, found in the role table, not named.
fn a_role_that_may_search() -> V2Role {
    V2_STAFF
        .into_iter()
        .find(|r| holds(*r, Permission::CustomerHoldPlace))
        .expect("premise: some staff role holds CustomerHoldPlace")
}

// ------------------------------------------------------------ the room

struct Room {
    db: TestDb,
}

struct Answer {
    status: StatusCode,
    body: Value,
    text: String,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

struct Seeded {
    somchai: String,
    malee: String,
}

const SOMCHAI_PASSPORT: &str = "AB1234567";
const MALEE_PASSPORT: &str = "X9876543";

impl Room {
    async fn start() -> Self {
        common::install_test_pii_keys();
        let db = TestDb::start().await;
        db.branch("B2").await;
        Self { db }
    }

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

    /// One LIVE customer, made the desk's way at `branch`; answers its id.
    async fn customer(
        &self,
        branch: &str,
        name: &str,
        passport: &str,
        nationality: &str,
    ) -> String {
        CustomerService::new(BranchTx::new(Transactor::new(self.db.pool.clone())))
            .reserve(
                &BranchId::new(branch),
                ReserveCustomer {
                    name: name.to_owned(),
                    passport_no: passport.to_owned(),
                    nationality: nationality.to_owned(),
                    birth_date: "1990-01-02".to_owned(),
                    passport_expiry: "2031-06-11".to_owned(),
                    mode: "LIVE".to_owned(),
                },
                Utc::now(),
            )
            .await
            .unwrap_or_else(|e| panic!("reserve {name}: {e:?}"))
            .customer_id
    }

    async fn seed(&self) -> Seeded {
        Seeded {
            somchai: self
                .customer("MAIN", "Somchai Jaidee", SOMCHAI_PASSPORT, "THA")
                .await,
            malee: self
                .customer("B2", "Malee Srisuk", MALEE_PASSPORT, "GBR")
                .await,
        }
    }

    async fn get(&self, who: &str, query: &str) -> Answer {
        let state = AppState::new(Arc::new(common::settings(&[])), self.db.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let uri = if query.is_empty() {
            format!("/api/v3/views/{VIEW}")
        } else {
            format!("/api/v3/views/{VIEW}?{query}")
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
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer { status, body, text }
    }

    /// The rows answered for `q` (URL-encoded here) by a person who may search.
    async fn rows(&self, who: &str, q: &str) -> Vec<Value> {
        let a = self.get(who, &format!("q={}", urlencoding(q))).await;
        assert_eq!(a.status, StatusCode::OK, "q={q:?}: {}", a.text);
        a.body["data"]
            .as_array()
            .unwrap_or_else(|| panic!("q={q:?}: data is the array of rows: {}", a.text))
            .clone()
    }

    async fn searcher(&self) -> String {
        let id = self.person("Searcher").await;
        format!("{}:{id}:MAIN", a_role_that_may_search().as_str())
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

/// Percent-encodes a query value (spaces, `-`, letters and digits are all a test needs).
fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn ids(rows: &[Value]) -> Vec<String> {
    let mut v: Vec<String> = rows
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .unwrap_or_else(|| panic!("a row with an id: {r}"))
                .to_owned()
        })
        .collect();
    v.sort();
    v
}

fn refused(a: &Answer, err: PolicyError, what: &str) {
    let code = match err {
        PolicyError::EmployeeAuthRequired => "employeeAuthRequired",
        PolicyError::RoleNotAllowed => "roleNotAllowed",
        other => panic!("not used here: {other:?}"),
    };
    assert_eq!(
        (a.status.as_u16(), a.code()),
        (err.status(), code),
        "{what}: {}",
        a.text
    );
    assert!(
        a.body["data"].is_null(),
        "{what}: no rows in a refusal: {}",
        a.text
    );
}

// ------------------------------------------------------------ who may search

#[test]
fn the_staff_customers_row_needs_customer_hold_place() {
    assert_eq!(
        Policy::need(EntryPath::StaffRpc, &Action::new("staffCustomers")),
        Some(Need::Permission(Permission::CustomerHoldPlace))
    );
}

#[actix_web::test]
async fn nobody_signed_in_is_asked_to_sign_in() {
    let room = Room::start().await;
    refused(
        &room.get("", "q=Somchai").await,
        PolicyError::EmployeeAuthRequired,
        "no session",
    );
    room.finish().await;
}

#[actix_web::test]
async fn each_staff_role_searches_exactly_when_it_holds_customer_hold_place() {
    let room = Room::start().await;
    let s = room.seed().await;
    assert!(
        V2_STAFF
            .iter()
            .any(|r| !holds(*r, Permission::CustomerHoldPlace)),
        "premise: some staff role lacks CustomerHoldPlace, so the refusal is exercised"
    );
    for role in V2_STAFF {
        let id = room.person(&format!("p-{}", role.as_str())).await;
        let who = format!("{}:{id}:MAIN", role.as_str());
        let a = room.get(&who, "q=Somchai%20Jaidee").await;
        if holds(role, Permission::CustomerHoldPlace) {
            assert_eq!(a.status, StatusCode::OK, "{}: {}", role.as_str(), a.text);
            assert!(
                a.text.contains(&s.somchai),
                "{}: finds the customer: {}",
                role.as_str(),
                a.text
            );
        } else {
            refused(&a, PolicyError::RoleNotAllowed, role.as_str());
        }
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_signed_in_person_without_customer_hold_place_is_refused() {
    let room = Room::start().await;
    room.seed().await;
    assert!(!RolePermissions::of(Role::Customer).contains(&Permission::CustomerHoldPlace));
    let id = room.person("A Customer").await;
    refused(
        &room.get(&format!("customer:{id}:MAIN"), "q=Somchai").await,
        PolicyError::RoleNotAllowed,
        "a CUSTOMER",
    );
    room.finish().await;
}

// ------------------------------------------------------------ nothing until a key is typed (S11)

#[actix_web::test]
async fn an_empty_or_missing_key_answers_an_empty_page() {
    let room = Room::start().await;
    room.seed().await;
    let who = room.searcher().await;
    for query in ["", "q=", "q=%20%20%20"] {
        let a = room.get(&who, query).await;
        assert_eq!(a.status, StatusCode::OK, "{query:?}: {}", a.text);
        assert_eq!(
            a.body["data"],
            Value::Array(vec![]),
            "{query:?}: nothing listed: {}",
            a.text
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_key_that_matches_nobody_answers_an_empty_page() {
    let room = Room::start().await;
    room.seed().await;
    let who = room.searcher().await;
    assert_eq!(room.rows(&who, "Nobody Here").await, Vec::<Value>::new());
    assert_eq!(room.rows(&who, "ZZ0000000").await, Vec::<Value>::new());
    room.finish().await;
}

// ------------------------------------------------------------ a name, a customer id, a passport number

#[actix_web::test]
async fn a_name_finds_the_customer_in_any_case_and_with_spaces_around_it() {
    let room = Room::start().await;
    let s = room.seed().await;
    let who = room.searcher().await;
    for q in [
        "Somchai Jaidee",
        "somchai jaidee",
        "SOMCHAI JAIDEE",
        "  Somchai Jaidee  ",
    ] {
        assert_eq!(
            ids(&room.rows(&who, q).await),
            vec![s.somchai.clone()],
            "q={q:?}"
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_customer_id_finds_the_customer_in_any_case() {
    let room = Room::start().await;
    let s = room.seed().await;
    let who = room.searcher().await;
    for q in [
        s.somchai.clone(),
        s.somchai.to_lowercase(),
        format!(" {} ", s.somchai),
    ] {
        assert_eq!(
            ids(&room.rows(&who, &q).await),
            vec![s.somchai.clone()],
            "q={q:?}"
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_passport_number_finds_the_customer_in_any_spacing_dashes_or_case() {
    let room = Room::start().await;
    let s = room.seed().await;
    let who = room.searcher().await;
    for q in [
        "AB1234567",
        "ab1234567",
        "AB 1234 567",
        "ab-1234-567",
        " AB1234567 ",
    ] {
        assert_eq!(
            ids(&room.rows(&who, q).await),
            vec![s.somchai.clone()],
            "q={q:?}"
        );
    }
    // the second customer, of another nationality, made at another branch: one customer per organisation
    for q in ["X9876543", "x 9876 543"] {
        assert_eq!(
            ids(&room.rows(&who, q).await),
            vec![s.malee.clone()],
            "q={q:?}"
        );
    }
    room.finish().await;
}

// ------------------------------------------------------------ what a row carries (#554 K8)

#[actix_web::test]
async fn a_row_is_exactly_id_and_display_name() {
    let room = Room::start().await;
    let s = room.seed().await;
    let who = room.searcher().await;
    let rows = room.rows(&who, "Somchai Jaidee").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = rows[0].as_object().expect("a row is an object");
    let mut keys: Vec<&str> = row.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, vec!["displayName", "id"], "{row:?}");
    assert_eq!(row["id"], Value::String(s.somchai.clone()));
    assert_eq!(
        row["displayName"],
        Value::String("Somchai Jaidee".to_owned())
    );
    room.finish().await;
}

#[actix_web::test]
async fn the_answer_never_carries_an_identity_value() {
    let room = Room::start().await;
    room.seed().await;
    let who = room.searcher().await;
    for q in [
        SOMCHAI_PASSPORT,
        "ab 1234 567",
        "Somchai Jaidee",
        MALEE_PASSPORT,
    ] {
        let a = room.get(&who, &format!("q={}", urlencoding(q))).await;
        assert_eq!(a.status, StatusCode::OK, "{}", a.text);
        let data = a.body["data"].to_string().to_uppercase();
        for secret in [
            SOMCHAI_PASSPORT,
            MALEE_PASSPORT,
            "1234567",
            "9876543",
            "4567",
            "6543",
        ] {
            assert!(
                !data.contains(secret),
                "q={q:?}: the answer must not carry an identity value or its masked form ({secret}): {data}"
            );
        }
        for nat in ["\"THA\"", "\"GBR\""] {
            assert!(
                !data.contains(nat),
                "q={q:?}: no nationality either: {data}"
            );
        }
    }
    room.finish().await;
}
