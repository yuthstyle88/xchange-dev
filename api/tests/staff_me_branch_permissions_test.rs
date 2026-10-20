//! Ticket #2695 — staff_me view lists permissions from the caller's roles on other branches,
//! not the effective ones at the named branch.
//!
//! Behaviour table rows S1–S5:
//! S1: TELLER@B1 + FINANCE@B2; branch=B1 -> exactly me_permissions at B1: no cashManage/financeReconcile
//! S2: same caller; branch=B2 -> FINANCE's permissions at B2, without TELLER-only ones
//! S3: single TELLER@B1; branch=B1 -> unchanged (kept: staff_me_answers_the_person_their_roles_branch_and_permissions)
//! S4: EXECUTIVE@*; any branch -> all its permissions (kept)
//! S5: any caller -> roles lists every role row (kept)

mod common;
mod lg4_fund;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use quote_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor as User, Role, RoleGrants, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const B1: &str = "B1";
const B2: &str = "B2";

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
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = header(req);
        Box::pin(async move {
            let Some((role, id, branch)) = parse(&spec) else {
                return Callers {
                    user: None,
                    staff_user: None,
                };
            };
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
            } else if role.contains('+') || role.contains('@') {
                let mut scoped_roles = Vec::new();
                for part in role.split('+') {
                    let (r_str, b_str) = part.split_once('@').unwrap_or((part, "*"));
                    let r = Role::parse(r_str).unwrap_or_else(|| panic!("unknown role {r_str}"));
                    let b = if b_str == "*" || b_str.is_empty() {
                        None
                    } else {
                        Some(BranchId::new(b_str))
                    };
                    scoped_roles.push(ScopedRole::new(r, b, since(), None));
                }
                Some(
                    User::new(&id, ORG_1, scoped_roles, None)
                        .with_branch(Some(BranchId::new(&branch))),
                )
            } else {
                test_callers::v2_staff_roles(&role).map(|roles| {
                    test_callers::staff(&id, roles, BranchId::new(&branch))
                        .with_branch(Some(BranchId::new(&branch)))
                })
            };
            Callers {
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

fn role_permissions(role: Role) -> Vec<String> {
    let mut ps: Vec<String> = RoleGrants::seed(role)
        .iter()
        .map(|p| {
            serde_json::to_string(p)
                .expect("a permission serialises")
                .trim_matches('"')
                .to_string()
        })
        .collect();
    ps.sort();
    ps.dedup();
    ps
}

struct Floor {
    db: TestDb,
}

struct Answer {
    status: StatusCode,
    body: Value,
}

impl Answer {
    fn data(&self) -> &Value {
        &self.body["data"]
    }
}

impl Floor {
    async fn start() -> Self {
        let db = TestDb::start().await;
        db.branch(B1).await;
        db.branch(B2).await;
        for branch in ["MAIN", B1, B2] {
            lg4_fund::lg4_fund_safe(&db.pool, branch).await;
        }
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

    async fn finish(self) {
        self.db.finish().await;
    }
}

/// S1: Caller with TELLER@B1 + FINANCE@B2 calling with branch=B1
/// Required result: exactly me_permissions at B1: no cashManage / financeReconcile.
#[actix_web::test]
async fn test_s1_teller_at_b1_and_finance_at_b2_queried_at_b1_excludes_finance_permissions() {
    let f = Floor::start().await;
    let id = f.person("DualStaff").await;
    let who = format!("TELLER@B1+FINANCE@B2:{id}:B1");

    let me = f.ok(&who, "staff_me", "branch=B1").await;
    assert_eq!(me["branchId"], json!("B1"));

    let permissions: Vec<String> = me["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    // S1 requirement: no cashManage or financeReconcile at B1
    assert!(
        !permissions.contains(&"cashManage".to_string()),
        "B1 should not include cashManage: got {permissions:?}"
    );
    assert!(
        !permissions.contains(&"financeReconcile".to_string()),
        "B1 should not include financeReconcile: got {permissions:?}"
    );

    // exactly me_permissions at B1 (matches TELLER's permissions)
    let expected = role_permissions(Role::Teller);
    assert_eq!(permissions, expected, "permissions at B1 must match TELLER");

    f.finish().await;
}

/// S2: Same caller (TELLER@B1 + FINANCE@B2) calling with branch=B2
/// Required result: FINANCE's permissions at B2, without TELLER-only ones.
#[actix_web::test]
async fn test_s2_teller_at_b1_and_finance_at_b2_queried_at_b2_excludes_teller_permissions() {
    let f = Floor::start().await;
    let id = f.person("DualStaff").await;
    let who = format!("TELLER@B1+FINANCE@B2:{id}:B2");

    let me = f.ok(&who, "staff_me", "branch=B2").await;
    assert_eq!(me["branchId"], json!("B2"));

    let permissions: Vec<String> = me["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    // TELLER-only permissions should not be present at B2
    assert!(
        !permissions.contains(&"exchangeTransact".to_string()),
        "B2 should not include exchangeTransact: got {permissions:?}"
    );
    assert!(
        !permissions.contains(&"bookingsDesk".to_string()),
        "B2 should not include bookingsDesk: got {permissions:?}"
    );

    // FINANCE permissions must be present at B2
    let expected = role_permissions(Role::Finance);
    assert_eq!(
        permissions, expected,
        "permissions at B2 must match FINANCE"
    );

    f.finish().await;
}

/// S3: Single TELLER@B1; branch=B1 -> unchanged (kept: answers TELLER's permissions at B1).
#[actix_web::test]
async fn test_s3_single_teller_at_b1_answers_teller_permissions() {
    let f = Floor::start().await;
    let id = f.person("TellerOnly").await;
    let who = format!("TELLER@B1:{id}:B1");

    let me = f.ok(&who, "staff_me", "branch=B1").await;
    assert_eq!(me["branchId"], json!("B1"));

    let permissions: Vec<String> = me["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    let expected = role_permissions(Role::Teller);
    assert_eq!(permissions, expected);

    f.finish().await;
}

/// S4: EXECUTIVE@*; any branch -> all its permissions (kept).
#[actix_web::test]
async fn test_s4_executive_org_wide_answers_all_permissions_at_any_branch() {
    let f = Floor::start().await;
    let id = f.person("ExecStaff").await;
    let who = format!("EXECUTIVE@*:{id}:B1");

    let me_b1 = f.ok(&who, "staff_me", "branch=B1").await;
    let permissions_b1: Vec<String> = me_b1["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    let expected = role_permissions(Role::Executive);
    assert_eq!(permissions_b1, expected);

    let me_b2 = f.ok(&who, "staff_me", "branch=B2").await;
    let permissions_b2: Vec<String> = me_b2["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    assert_eq!(permissions_b2, expected);

    f.finish().await;
}

/// S5: Any caller -> roles lists every role row (kept).
#[actix_web::test]
async fn test_s5_roles_lists_every_role_row() {
    let f = Floor::start().await;
    let id = f.person("MultiRoleStaff").await;
    let who = format!("TELLER@B1+FINANCE@B2:{id}:B1");

    let me = f.ok(&who, "staff_me", "branch=B1").await;

    let roles = me["roles"].as_array().expect("roles array");
    assert_eq!(roles.len(), 2, "roles must list every role row: {roles:?}");

    let mut role_tuples: Vec<(String, Option<String>)> = roles
        .iter()
        .map(|r| {
            (
                r["role"].as_str().unwrap().to_string(),
                r["branchId"].as_str().map(|s| s.to_string()),
            )
        })
        .collect();
    role_tuples.sort();

    let mut expected_roles = vec![
        ("TELLER".to_string(), Some("B1".to_string())),
        ("FINANCE".to_string(), Some("B2".to_string())),
    ];
    expected_roles.sort();

    assert_eq!(role_tuples, expected_roles);

    f.finish().await;
}
