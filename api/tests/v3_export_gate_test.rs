//! Ticket #1147 (EX-9): an export of a paged view is gated by `ReportsExport` and audited.
//! Written before the implementation by the up-front test writer (claude/709ec749, on the BE lane controller
//! claude/ad05bf8d's order; the ticket author claude/89bd61d9 delegated it). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Design: `docs/design/blueprint/executive-finance.md` §8.4 (D-8), §9 (`ExportGate`), §11.1 EX-9; owner decision E-2
//! (only EXECUTIVE and FINANCE export, through `ReportsExport`; an export carries no identity-document number).
//! Ticket #1147 §3, §7, §8 and §9; the lane controller's ruling issuecomment-6100577886: the gate runs right after
//! the view's own `Policy::authorize`, against the branch the view resolved (`grant.branch`), before any row is read.
//!
//! What this file pins ([required] in ticket §7/§8 unless marked "choice"):
//! - The query key `purpose`; its only value is `export`. Any other value, or two different values → `400
//!   invalidRequest`, field `purpose`. A branch view (here `me`) refuses `purpose` the same way.
//! - `GET /api/v3/views/<paged view>?…&purpose=export`: authenticate; the view's own check (unchanged); the gate
//!   (`Authorizer::check(actor, ReportsExport, the resolved branch)`); then the read. Refused by the gate → a client
//!   error with `error.code = "outOfScope"` (ticket §8; asserted as a client error plus the code, not a hard-coded
//!   status), no rows, no audit row.
//! - Rows answered → exactly one `audit_events` row per request: `actor_id` = the user id, `actor_role` = the first
//!   role of the export grant (`FINANCE`, `EXECUTIVE`), `scope` `BRANCH`, `branch_id` the resolved branch, `action`
//!   `EXPORT`, `object_ref` `view:<NAME>`, `before_redacted` NULL, `after_redacted` `{"filter": {<query keys other
//!   than branch, since, before, limit, purpose, as decoded strings, through the shared Redactor>}, "rows": <count>}`.
//! - No rows leave the server (`since` = the version) → no audit row. The audit append fails → `500 serverError`
//!   and no `data` (fail closed).
//! - Without `purpose`, a read is exactly what it was, and writes no audit row.
//!
//! Callers are Blueprint `access::Actor`s handed to the seam through `SessionReader::callers` (`user`), built from
//! the header `x-test-as`: `ROLE@BRANCH` items joined by `,`, `ROLE@*` for an organisation-wide row, and a trailing
//! `~` on an item for a row whose `valid_to` has passed. The seam also wants a legacy session; a v2 FRONT session
//! at the first item's branch stands in (choice; OR-7.7 #1030 removes that need). Roles only *make* actors.
//! The view under test is `admin_transactions` (`EntryPath::Rpc`, `adminSnapshot` → `CompatRead`), rows seeded with
//! SQL as LIVE completed exchanges. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, Utc};
use quote_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor as User, Permission, Role, RoleGrants, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor as V2Actor;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const VIEW: &str = "admin_transactions";
const USER: &str = "00000000-0000-4000-8000-0000000e9001";
/// The teller of the seeded shifts (`shifts.teller_id` is a user).
const TELLER: &str = "00000000-0000-4000-8000-0000000e9002";

// ------------------------------------------------------------ the caller

fn since() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// `ROLE@BRANCH[,ROLE@BRANCH…]`; `*` for an organisation-wide row; a trailing `~` for an expired row.
fn actor_of(spec: &str) -> Option<(User, String)> {
    if spec.is_empty() {
        return None;
    }
    let mut roles = Vec::new();
    let mut first_branch = None;
    for item in spec.split(',') {
        let (item, expired) = match item.strip_suffix('~') {
            Some(i) => (i, true),
            None => (item, false),
        };
        let (role, branch) = item.split_once('@').expect("ROLE@BRANCH");
        let role = Role::parse(role).expect("a known role");
        let branch = if branch == "*" {
            None
        } else {
            first_branch.get_or_insert_with(|| branch.to_owned());
            Some(BranchId::new(branch))
        };
        let to = expired.then(|| Utc::now() - Duration::days(1));
        roles.push(ScopedRole::new(role, branch, since(), to));
    }
    let home = first_branch.unwrap_or_else(|| "MAIN".to_owned());
    // a session always has a branch (SB-1): the first item's, as the sign-in would pick
    let user = User::new(USER, ORG_1, roles, None).with_branch(Some(BranchId::new(&home)));
    Some((user, home))
}

struct HeaderSessions;

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
            // the seam's legacy actor (OR-7.7 #1030 retires it): an owner session, which reads any branch, so the
            // Blueprint actor alone decides who may read and export
            let owner = actor_of(&spec).map(|_| V2Actor::owner(USER));
            Sessions { owner, staff: None }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = header(req);
        Box::pin(async move {
            let _s = self.sessions(req).await;
            let user = actor_of(&spec).map(|(u, _)| u);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
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
    fn rows(&self) -> &Vec<Value> {
        self.body["data"]
            .as_array()
            .unwrap_or_else(|| panic!("data is the array of rows: {}", self.text))
    }
}

impl Room {
    /// Branches B1 and B3 of the organisation, three LIVE completed exchanges at B1 and one at B3.
    async fn start() -> Self {
        common::install_test_pii_keys();
        let db = TestDb::start().await;
        db.branch("B1").await;
        db.branch("B3").await;
        db.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{TELLER}', '{ORG_1}', 'A teller', 'ACTIVE', 'test')"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('CX-00000001', '{ORG_1}', 'CX-00000001', 'Somchai Jaidee', 'LIVE', now())"
        ))
        .await;
        for b in ["B1", "B3"] {
            db.exec(&format!(
                "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, closed_at, \
                    closed_by, teller_id) \
                 VALUES ('SH-{b}', '{b}', 'CLOSED', now(), 'test', 'open-{b}', now(), 'test', '{TELLER}')"
            ))
            .await;
        }
        let room = Self { db };
        for n in 1..=3 {
            room.exchange("B1", n).await;
        }
        room.exchange("B3", 4).await;
        room
    }

    async fn exchange(&self, branch: &str, n: u32) {
        let id = uuid::Uuid::new_v4();
        let at = format!("2026-10-06T05:0{n}:00Z");
        self.db
            .exec(&format!(
                "INSERT INTO transactions (id, document_ref, branch_id, shift_id, mode, submission_id, payload_hash, customer_id, \
                    customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
                    recorded_by, status, completed_at, organization_id) \
                 VALUES ('{id}', 'XC-{branch}-{n:06}', '{branch}', 'SH-{branch}', 'LIVE', 'sub-{id}', 'hash', \
                    'CX-00000001', 'Somchai', 'THA', 'EMP-001', 'WALK-IN', '', '2026-10-06', '{at}', '{at}', \
                    'owner', 'COMPLETED', '{at}', (SELECT organization_id FROM branches WHERE id = '{branch}'))"
            ))
            .await;
        // the view joins line 1 and payment 1 (`dto/v3/transaction_row.rs` ROW_FROM)
        self.db
            .exec(&format!(
                "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, quantity, \
                    unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
                 VALUES (gen_random_uuid(), '{ORG_1}', '{id}', 'BUY', 'USD', 100, 1, 32.1, 3210.00, 10.00, \
                    3200.00, 1, now(), now())"
            ))
            .await;
        self.db
            .exec(&format!(
                "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, \
                    reference, account, amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
                 VALUES (gen_random_uuid(), '{ORG_1}', '{id}', 1, 'Cash', '', '', 3200.00, 0, '{at}', now(), \
                    now())"
            ))
            .await;
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
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer { status, body, text }
    }

    /// Every `audit_events` row, oldest first, as `(actor_id, actor_role, scope, branch_id, action, object_ref,
    /// before_redacted, after_redacted)` in one JSON object each.
    async fn audit(&self) -> Vec<Value> {
        sqlx::query_scalar::<_, Value>(
            "SELECT json_build_object('actor_id', actor_id, 'actor_role', actor_role, 'scope', scope, \
                'branch_id', branch_id, 'action', action, 'object_ref', object_ref, \
                'before_redacted', before_redacted, 'after_redacted', after_redacted)::jsonb \
             FROM audit_events ORDER BY id",
        )
        .fetch_all(&self.db.pool)
        .await
        .expect("read audit_events")
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

fn refused(a: &Answer, code: &str, what: &str) {
    assert!(
        a.status.is_client_error(),
        "{what}: a client error, got {}: {}",
        a.status,
        a.text
    );
    assert_eq!(a.code(), code, "{what}: {}", a.text);
    assert!(
        a.body["data"].is_null(),
        "{what}: no rows in a refusal: {}",
        a.text
    );
}

fn invalid_purpose(a: &Answer, what: &str) {
    refused(a, "invalidRequest", what);
    assert_eq!(
        a.body["error"]["field"],
        json!("purpose"),
        "{what}: {}",
        a.text
    );
}

fn holds(role: Role, p: Permission) -> bool {
    RoleGrants::seed(role).contains(&p)
}

// ------------------------------------------------------------ who may export (E-2)

#[actix_web::test]
async fn finance_at_b1_exports_b1_and_one_audit_row_records_it() {
    assert!(
        holds(Role::Finance, Permission::ReportsExport),
        "premise E-2"
    );
    let room = Room::start().await;
    let a = room
        .get("FINANCE@B1", VIEW, "branch=B1&purpose=export")
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.rows().len(), 3, "B1's three exchanges: {}", a.text);
    assert_eq!(
        room.audit().await,
        vec![json!({
            "actor_id": USER,
            "actor_role": "FINANCE",
            "scope": "BRANCH",
            "branch_id": "B1",
            "action": "EXPORT",
            "object_ref": "view:admin_transactions",
            "before_redacted": null,
            "after_redacted": { "filter": {}, "rows": 3 },
        })],
        "exactly one audit row, as ticket §8"
    );
    room.finish().await;
}

#[actix_web::test]
async fn an_organisation_wide_executive_exports_a_named_branch_with_one_audit_row() {
    assert!(
        holds(Role::Executive, Permission::ReportsExport),
        "premise E-2"
    );
    let room = Room::start().await;
    let a = room
        .get("EXECUTIVE@*", VIEW, "branch=B1&purpose=export")
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.rows().len(), 3, "{}", a.text);
    let audit = room.audit().await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0]["actor_role"], json!("EXECUTIVE"));
    assert_eq!(audit[0]["branch_id"], json!("B1"));
    assert_eq!(audit[0]["after_redacted"]["rows"], json!(3));
    room.finish().await;
}

#[actix_web::test]
async fn a_reader_without_reports_export_is_refused_out_of_scope_and_nothing_is_audited() {
    let room = Room::start().await;
    for role in [Role::Teller, Role::BranchManager] {
        assert!(
            holds(role, Permission::CompatRead),
            "premise: {} reads the view",
            role.as_str()
        );
        assert!(
            !holds(role, Permission::ReportsExport),
            "premise E-2: {}",
            role.as_str()
        );
        let who = format!("{}@B1", role.as_str());
        let a = room.get(&who, VIEW, "branch=B1&purpose=export").await;
        refused(&a, "outOfScope", role.as_str());
    }
    assert_eq!(
        room.audit().await,
        Vec::<Value>::new(),
        "no audit row for a refusal"
    );
    room.finish().await;
}

#[actix_web::test]
async fn without_purpose_a_read_is_unchanged_and_writes_no_audit_row() {
    let room = Room::start().await;
    let plain = room.get("TELLER@B1", VIEW, "branch=B1").await;
    assert_eq!(plain.status, StatusCode::OK, "{}", plain.text);
    assert_eq!(plain.rows().len(), 3, "{}", plain.text);
    let exported = room
        .get("FINANCE@B1", VIEW, "branch=B1&purpose=export")
        .await;
    assert_eq!(
        exported.body["data"], plain.body["data"],
        "an export answers the same rows as the read"
    );
    let before = room.audit().await.len();
    let again = room.get("FINANCE@B1", VIEW, "branch=B1").await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(
        room.audit().await.len(),
        before,
        "a read without purpose is not audited"
    );
    room.finish().await;
}

#[actix_web::test]
async fn the_gate_checks_the_resolved_branch_not_any_branch_the_user_may_export() {
    let room = Room::start().await;
    let who = "FINANCE@B1,TELLER@B3";
    let read = room.get(who, VIEW, "branch=B3").await;
    assert_eq!(
        read.status,
        StatusCode::OK,
        "B3 is readable as TELLER: {}",
        read.text
    );
    assert_eq!(read.rows().len(), 1, "{}", read.text);
    refused(
        &room.get(who, VIEW, "branch=B3&purpose=export").await,
        "outOfScope",
        "FINANCE at B1 exporting B3",
    );
    assert_eq!(room.audit().await, Vec::<Value>::new());
    let own = room.get(who, VIEW, "branch=B1&purpose=export").await;
    assert_eq!(
        own.status,
        StatusCode::OK,
        "B1 is still exportable: {}",
        own.text
    );
    room.finish().await;
}

#[actix_web::test]
async fn an_expired_reports_export_role_does_not_export() {
    let room = Room::start().await;
    let who = "TELLER@B1,FINANCE@B1~";
    let read = room.get(who, VIEW, "branch=B1").await;
    assert_eq!(
        read.status,
        StatusCode::OK,
        "the TELLER row still reads: {}",
        read.text
    );
    refused(
        &room.get(who, VIEW, "branch=B1&purpose=export").await,
        "outOfScope",
        "FINANCE whose row has expired",
    );
    assert_eq!(room.audit().await, Vec::<Value>::new());
    room.finish().await;
}

// ------------------------------------------------------------ one audit row per request

#[actix_web::test]
async fn a_three_page_walk_writes_three_audit_rows_one_per_page() {
    let room = Room::start().await;
    let first = room
        .get("FINANCE@B1", VIEW, "branch=B1&limit=1&purpose=export")
        .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    assert_eq!(first.rows().len(), 1, "{}", first.text);
    let mut before = first.rows()[0]["id"].as_str().expect("a row id").to_owned();
    for page in 2..=3 {
        let a = room
            .get(
                "FINANCE@B1",
                VIEW,
                &format!("branch=B1&limit=1&before={before}&purpose=export"),
            )
            .await;
        assert_eq!(a.status, StatusCode::OK, "page {page}: {}", a.text);
        assert_eq!(a.rows().len(), 1, "page {page}: {}", a.text);
        before = a.rows()[0]["id"].as_str().expect("a row id").to_owned();
    }
    let audit = room.audit().await;
    assert_eq!(audit.len(), 3, "one audit row per request: {audit:?}");
    for row in &audit {
        assert_eq!(
            row["after_redacted"],
            json!({ "filter": {}, "rows": 1 }),
            "{row}"
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_walk_that_starts_mid_way_is_audited_too() {
    let room = Room::start().await;
    let all = room.get("FINANCE@B1", VIEW, "branch=B1").await;
    let mid = all.rows()[0]["id"].as_str().expect("a row id").to_owned();
    let a = room
        .get(
            "FINANCE@B1",
            VIEW,
            &format!("branch=B1&before={mid}&purpose=export"),
        )
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.rows().len(), 2, "{}", a.text);
    let audit = room.audit().await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0]["after_redacted"]["rows"], json!(2));
    room.finish().await;
}

#[actix_web::test]
async fn an_unchanged_answer_sends_no_rows_and_writes_no_audit_row() {
    let room = Room::start().await;
    let first = room.get("FINANCE@B1", VIEW, "branch=B1").await;
    let version = first.body["version"].clone();
    assert!(version.is_number(), "{}", first.text);
    let a = room
        .get(
            "FINANCE@B1",
            VIEW,
            &format!("branch=B1&since={version}&purpose=export"),
        )
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["changed"], json!(false), "{}", a.text);
    assert!(
        a.body.get("data").is_none() || a.body["data"].is_null(),
        "{}",
        a.text
    );
    assert_eq!(room.audit().await, Vec::<Value>::new());
    room.finish().await;
}

// ------------------------------------------------------------ the wire contract of purpose

#[actix_web::test]
async fn purpose_other_than_export_is_an_invalid_request() {
    let room = Room::start().await;
    invalid_purpose(
        &room
            .get("FINANCE@B1", VIEW, "branch=B1&purpose=download")
            .await,
        "download",
    );
    invalid_purpose(
        &room
            .get(
                "FINANCE@B1",
                VIEW,
                "branch=B1&purpose=export&purpose=download",
            )
            .await,
        "two different values",
    );
    assert_eq!(room.audit().await, Vec::<Value>::new());
    room.finish().await;
}

#[actix_web::test]
async fn a_branch_view_refuses_purpose() {
    let room = Room::start().await;
    let plain = room.get("FINANCE@B1", "me", "").await;
    assert_eq!(
        plain.status,
        StatusCode::OK,
        "premise: me is readable: {}",
        plain.text
    );
    invalid_purpose(
        &room.get("FINANCE@B1", "me", "purpose=export").await,
        "me with purpose",
    );
    room.finish().await;
}

#[actix_web::test]
async fn purpose_is_not_a_filter_key_and_the_audit_row_carries_the_other_keys_through_the_redactor()
{
    let room = Room::start().await;
    let a = room
        .get(
            "FINANCE@B1",
            VIEW,
            "branch=B1&q=Somchai%20J&phone=0812345678&purpose=export",
        )
        .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "purpose is never read as a filter key: {}",
        a.text
    );
    let audit = room.audit().await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(
        audit[0]["after_redacted"]["filter"],
        json!({ "q": "Somchai J", "phone": "***5678" }),
        "decoded strings, the phone masked by the shared Redactor, and no branch/purpose"
    );
    assert_eq!(audit[0]["after_redacted"]["rows"], json!(a.rows().len()));
    room.finish().await;
}

#[actix_web::test]
async fn the_audit_row_never_carries_row_content() {
    let room = Room::start().await;
    let a = room
        .get("FINANCE@B1", VIEW, "branch=B1&purpose=export")
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let audit = room.audit().await;
    assert_eq!(audit.len(), 1, "one audit row: {audit:?}");
    let after = audit[0]["after_redacted"].to_string();
    for row in a.rows() {
        let id = row["id"].as_str().expect("a row id");
        assert!(!after.contains(id), "no row id in the audit row: {after}");
    }
    assert!(!after.contains("Somchai"), "no customer name: {after}");
    assert!(!after.contains("XC-B1-"), "no document number: {after}");
    room.finish().await;
}

// ------------------------------------------------------------ fail closed

#[actix_web::test]
async fn when_the_audit_cannot_be_written_no_rows_are_answered() {
    let room = Room::start().await;
    room.db.exec("DROP TABLE audit_events").await;
    let a = room
        .get("FINANCE@B1", VIEW, "branch=B1&purpose=export")
        .await;
    assert_eq!(a.status, StatusCode::INTERNAL_SERVER_ERROR, "{}", a.text);
    assert_eq!(a.code(), "serverError", "{}", a.text);
    assert!(a.body["data"].is_null(), "fail closed: no rows: {}", a.text);
    let plain = room.get("FINANCE@B1", VIEW, "branch=B1").await;
    assert_eq!(
        plain.status,
        StatusCode::OK,
        "a read without purpose does not need the audit: {}",
        plain.text
    );
    room.finish().await;
}

#[actix_web::test]
async fn the_audit_row_names_the_role_that_granted_the_export_not_another_role_held() {
    let room = Room::start().await;
    let a = room
        .get("FINANCE@B1,TELLER@B1", VIEW, "branch=B1&purpose=export")
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let audit = room.audit().await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(
        audit[0]["actor_role"],
        json!("FINANCE"),
        "the export grant's role, not TELLER"
    );
    room.finish().await;
}
