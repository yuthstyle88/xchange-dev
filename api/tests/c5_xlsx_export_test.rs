//! C5 (`docs/design/blueprint/user-management-rbac.md` §3.6, §7; BE controller rulings #564 issuecomment-6245633900
//! and the cursor ruling A): an xlsx form of a paged view's export, `?purpose=export&format=xlsx`, through the
//! existing `ExportGate`.
//!
//! Written before the implementation by the ticket author (claude/dell-claude-5). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! What this file pins:
//! - Without `format`, a read and a `purpose=export` read answer exactly today's JSON bytes (`view.rs:739-790`).
//! - `format=xlsx` is valid only with `purpose=export`; any other value, or `format` without the export ⇒ `400
//!   invalidRequest` on `format`, and no audit row.
//! - `format=xlsx` answers one workbook of **every** row: the server walks the pages itself with `before` = the
//!   last row's `CURSOR_FIELD` (`admin_users`: `userId`), `limit` 100. One sheet named `admin_users`; the header is
//!   the row's serde field names in order; `since` does not apply. `200`, the xlsx MIME type, `Cache-Control:
//!   no-store`, `Content-Disposition: attachment; filename="admin_users-<branch>-<YYYYMMDDTHHMMSSZ>.xlsx"`.
//! - Exactly one `audit_events` row per xlsx export, with the total row count.
//! - Strings are string cells, never formulas; numbers are numbers; an array is its JSON text; `null` is empty.
//! - More than 50 000 rows ⇒ `413 exportTooLarge`, no workbook, no audit row; exactly 50 000 ⇒ `200`.
//! - The gate is unchanged: a caller without `ReportsExport` is refused `outOfScope` and nothing is audited.
//!
//! Callers are built from the header `x-test-as` (`ROLE@BRANCH` items joined by `,`, `ROLE@*` organisation-wide), as
//! `tests/v3_export_gate_test.rs` does. `admin_users` needs `UsersManage` (ADMIN) and an export needs
//! `ReportsExport` (EXECUTIVE, FINANCE), so the exporter is the owner, EXECUTIVE + ADMIN (B3). Needs
//! `TEST_DATABASE_URL`; nothing skips.

mod common;
mod quote_common;

use std::future::Future;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{App, HttpRequest, test as atest, web};
use calamine::{Data, Reader, Xlsx, open_workbook_from_rs};
use chrono::{DateTime, Utc};
use quote_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const VIEW: &str = "admin_users";
const USER: &str = "00000000-0000-4000-8000-0000000c5001";
const OWNER: &str = "EXECUTIVE@*,ADMIN@*";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const HEADER: [&str; 9] = [
    "userId",
    "name",
    "employeeCode",
    "department",
    "position",
    "tellCode",
    "status",
    "version",
    "roles",
];

// ------------------------------------------------------------ the caller

fn actor_of(spec: &str) -> Option<User> {
    if spec.is_empty() {
        return None;
    }
    let mut roles = Vec::new();
    for item in spec.split(',') {
        let (role, branch) = item.split_once('@').expect("ROLE@BRANCH");
        let role = Role::parse(role).expect("a known role");
        let branch = (branch != "*").then(|| BranchId::new(branch));
        roles.push(ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        ));
    }
    Some(User::new(USER, ORG_1, roles, None).with_branch(Some(BranchId::new("B1"))))
}

struct HeaderSessions;

fn header(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("an ASCII header").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = header(req);
        Box::pin(async move { test_callers::callers(None, actor_of(&spec)) })
    }
}

// ------------------------------------------------------------ the room

struct Room {
    db: TestDb,
}

struct Answer {
    status: StatusCode,
    content_type: String,
    cache_control: String,
    disposition: String,
    bytes: Vec<u8>,
    body: Value,
    text: String,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    /// The workbook's sheet names and the rows of its first sheet, header included.
    fn workbook(&self) -> (Vec<String>, Vec<Vec<Data>>) {
        let mut wb: Xlsx<_> = open_workbook_from_rs(Cursor::new(self.bytes.clone()))
            .unwrap_or_else(|e| {
                panic!(
                    "the answer is an xlsx workbook ({e}): {} bytes",
                    self.bytes.len()
                )
            });
        let names = wb.sheet_names();
        let first = names.first().cloned().expect("at least one sheet");
        let range = wb.worksheet_range(&first).expect("the first sheet");
        let rows = range.rows().map(|r| r.to_vec()).collect();
        (names, rows)
    }
}

impl Room {
    /// Branch B1 of the first organisation, and the users named `names` (no roles).
    async fn start(names: &[&str]) -> Self {
        common::install_test_pii_keys();
        let db = TestDb::start().await;
        db.branch("B1").await;
        let room = Self { db };
        for name in names {
            room.user(name).await;
        }
        room
    }

    async fn user(&self, name: &str) {
        let name = name.replace('\'', "''");
        self.db
            .exec(&format!(
                "INSERT INTO users (organization_id, name, status, created_by) \
                 VALUES ('{ORG_1}', '{name}', 'ACTIVE', 'test')"
            ))
            .await;
    }

    /// `n` more users of the first organisation, named `bulk-000001` … in one statement.
    async fn bulk_users(&self, from: i64, n: i64) {
        self.db
            .exec(&format!(
                "INSERT INTO users (organization_id, name, status, created_by) \
                 SELECT '{ORG_1}', 'bulk-' || lpad(g::text, 6, '0'), 'ACTIVE', 'test' \
                 FROM generate_series({from}, {}) AS g",
                from + n - 1
            ))
            .await;
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
        let mut req = atest::TestRequest::get()
            .uri(&format!("/api/v3/views/{VIEW}?{query}"))
            .insert_header(("host", HOST))
            .insert_header(("x-xchange-request", "1"));
        if !who.is_empty() {
            req = req.insert_header((AS, who));
        }
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let h = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned()
        };
        let content_type = h("content-type");
        let cache_control = h("cache-control");
        let disposition = h("content-disposition");
        let bytes = atest::read_body(resp).await.to_vec();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer {
            status,
            content_type,
            cache_control,
            disposition,
            bytes,
            body,
            text,
        }
    }

    /// `(action, object_ref, after_redacted)` of every `audit_events` row, oldest first.
    async fn audit(&self) -> Vec<Value> {
        sqlx::query_scalar::<_, Value>(
            "SELECT json_build_object('action', action, 'object_ref', object_ref, \
                'after_redacted', after_redacted)::jsonb FROM audit_events ORDER BY id",
        )
        .fetch_all(&self.db.pool)
        .await
        .expect("read audit_events")
    }

    /// `(id, name, status, version)` of every user of the first organisation, in the view's order.
    async fn users(&self) -> Vec<(String, String, String, i32)> {
        sqlx::query_as(
            "SELECT id::text, name, status, version FROM users \
             WHERE organization_id = $1::uuid ORDER BY name, id",
        )
        .bind(ORG_1)
        .fetch_all(&self.db.pool)
        .await
        .expect("read users")
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

fn header_row(rows: &[Vec<Data>]) -> Vec<String> {
    rows.first()
        .expect("a header row")
        .iter()
        .map(|c| c.to_string())
        .collect()
}

fn refused_on_format(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(a.code(), "invalidRequest", "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["field"],
        json!("format"),
        "{what}: {}",
        a.text
    );
}

// ------------------------------------------------------------ the JSON path is unchanged

#[actix_web::test]
async fn a_read_without_purpose_answers_exactly_todays_json_bytes() {
    let room = Room::start(&["Alice", "Bob", "Chai"]).await;
    let a = room.get(OWNER, "branch=B1").await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.content_type, "application/json", "{}", a.text);
    assert_eq!(a.cache_control, "no-store", "{}", a.text);
    let version = a.body["version"].as_i64().expect("a version");
    let rows: Vec<String> = room
        .users()
        .await
        .into_iter()
        .map(|(id, name, status, v)| {
            format!(r#"{{"userId":"{id}","name":"{name}","employeeCode":null,"department":null,"position":null,"tellCode":null,"status":"{status}","version":{v},"roles":[]}}"#)
        })
        .collect();
    let expected = format!(
        r#"{{"version":{version},"changed":true,"data":[{}]}}"#,
        rows.join(",")
    );
    assert_eq!(a.text, expected, "the JSON bytes of a plain read");
    assert!(
        room.audit().await.is_empty(),
        "a plain read writes no audit row"
    );
    room.finish().await;
}

#[actix_web::test]
async fn an_export_without_format_answers_the_same_json_bytes_and_one_audit_row() {
    let room = Room::start(&["Alice", "Bob"]).await;
    let plain = room.get(OWNER, "branch=B1").await;
    let export = room.get(OWNER, "branch=B1&purpose=export").await;
    assert_eq!(export.status, StatusCode::OK, "{}", export.text);
    assert_eq!(export.content_type, "application/json", "{}", export.text);
    assert_eq!(
        export.text, plain.text,
        "purpose=export without format is today's JSON"
    );
    let audit = room.audit().await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0]["after_redacted"]["rows"], json!(2), "{audit:?}");
    room.finish().await;
}

// ------------------------------------------------------------ format is a core key

#[actix_web::test]
async fn format_without_purpose_export_is_an_invalid_request_on_format() {
    let room = Room::start(&["Alice"]).await;
    refused_on_format(
        &room.get(OWNER, "branch=B1&format=xlsx").await,
        "format without purpose",
    );
    assert!(room.audit().await.is_empty());
    room.finish().await;
}

#[actix_web::test]
async fn a_format_other_than_xlsx_is_an_invalid_request_on_format() {
    let room = Room::start(&["Alice"]).await;
    for f in ["csv", "XLSX", ""] {
        refused_on_format(
            &room
                .get(OWNER, &format!("branch=B1&purpose=export&format={f}"))
                .await,
            &format!("format={f:?}"),
        );
    }
    assert!(room.audit().await.is_empty());
    room.finish().await;
}

// ------------------------------------------------------------ the workbook

#[actix_web::test]
async fn an_xlsx_export_is_one_sheet_of_every_row_across_pages_with_the_attachment_headers() {
    // 230 users ⇒ three pages of 100, walked by the server with `before` = the last row's `userId`
    let room = Room::start(&[]).await;
    room.bulk_users(1, 230).await;
    let started = Utc::now();
    let a = room
        .get(OWNER, "branch=B1&purpose=export&format=xlsx")
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.content_type, XLSX);
    assert_eq!(a.cache_control, "no-store");

    // attachment; filename="admin_users-B1-<YYYYMMDDTHHMMSSZ>.xlsx", the time UTC and not before the request
    let prefix = "attachment; filename=\"admin_users-B1-";
    assert!(a.disposition.starts_with(prefix), "{}", a.disposition);
    let stamp = a
        .disposition
        .strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(".xlsx\""))
        .unwrap_or_else(|| panic!("{}", a.disposition));
    let at = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ")
        .unwrap_or_else(|e| panic!("{stamp}: {e}"))
        .and_utc();
    assert!(
        at >= started - chrono::Duration::seconds(1),
        "{stamp} vs {started}"
    );

    let (names, rows) = a.workbook();
    assert_eq!(
        names,
        vec![VIEW.to_owned()],
        "one sheet named after the view"
    );
    assert_eq!(
        header_row(&rows),
        HEADER,
        "the header is the serde field names in order"
    );
    let expected: Vec<String> = room.users().await.into_iter().map(|u| u.0).collect();
    let got: Vec<String> = rows[1..].iter().map(|r| r[0].to_string()).collect();
    assert_eq!(got.len(), 230, "every row, not one page");
    assert_eq!(got, expected, "every user once, in the view's order");
    room.finish().await;
}

#[actix_web::test]
async fn one_xlsx_export_writes_exactly_one_audit_row_with_the_total() {
    let room = Room::start(&[]).await;
    room.bulk_users(1, 230).await;
    let a = room
        .get(OWNER, "branch=B1&purpose=export&format=xlsx")
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let audit = room.audit().await;
    assert_eq!(
        audit.len(),
        1,
        "one audit row per export, not one per page: {audit:?}"
    );
    assert_eq!(audit[0]["action"], json!("EXPORT"), "{audit:?}");
    assert_eq!(
        audit[0]["object_ref"],
        json!("view:admin_users"),
        "{audit:?}"
    );
    assert_eq!(audit[0]["after_redacted"]["rows"], json!(230), "{audit:?}");
    room.finish().await;
}

#[actix_web::test]
async fn cells_keep_their_kinds_and_a_string_is_never_a_formula() {
    let room = Room::start(&["=1+1", "@SUM(A1)"]).await;
    let a = room
        .get(OWNER, "branch=B1&purpose=export&format=xlsx")
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let (_, rows) = a.workbook();
    let by_name: Vec<&Vec<Data>> = rows[1..].iter().collect();
    assert_eq!(by_name.len(), 2);
    for row in &by_name {
        assert!(
            matches!(row[1], Data::String(_)),
            "name is a string cell: {:?}",
            row[1]
        );
        assert!(
            matches!(row[7], Data::Float(_) | Data::Int(_)),
            "version is a number cell: {:?}",
            row[7]
        );
        assert_eq!(
            row[8],
            Data::String("[]".to_owned()),
            "roles is its JSON text"
        );
    }
    let names: Vec<String> = by_name.iter().map(|r| r[1].to_string()).collect();
    assert_eq!(names, vec!["=1+1".to_owned(), "@SUM(A1)".to_owned()]);
    // no cell of the sheet is a formula
    let mut wb: Xlsx<_> = open_workbook_from_rs(Cursor::new(a.bytes.clone())).expect("xlsx");
    let formulas = wb.worksheet_formula(VIEW).expect("the sheet's formulas");
    assert!(
        formulas.used_cells().all(|(_, _, f)| f.is_empty()),
        "a string is written as text, never as a formula"
    );
    room.finish().await;
}

#[derive(serde::Serialize)]
struct Sample {
    zeta: bool,
    alpha: Option<String>,
    mid: Value,
    num: f64,
    text: String,
}

#[test]
fn the_writer_keeps_field_order_and_maps_booleans_nulls_and_objects() {
    use xchange::presentation::http::v3::xlsx::{ordered_row, workbook};
    let samples = [
        Sample {
            zeta: true,
            alpha: None,
            mid: json!({"k": 1}),
            num: 2.5,
            text: "x".to_owned(),
        },
        Sample {
            zeta: false,
            alpha: Some("y".to_owned()),
            mid: json!([1, 2]),
            num: -3.0,
            text: String::new(),
        },
    ];
    let rows: Vec<_> = samples
        .iter()
        .map(|s| ordered_row(s).expect("an object"))
        .collect();
    assert_eq!(
        rows[0].iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        ["zeta", "alpha", "mid", "num", "text"],
        "ordered_row keeps the serialisation order, not the alphabet"
    );
    assert!(
        ordered_row(&json!(5)).is_err(),
        "a row that is not an object is an error"
    );
    let bytes = workbook("s", &rows).expect("a workbook");
    let mut wb: Xlsx<_> = open_workbook_from_rs(Cursor::new(bytes)).expect("an xlsx workbook");
    assert_eq!(wb.sheet_names(), vec!["s".to_owned()]);
    let range = wb.worksheet_range("s").expect("sheet s");
    let got: Vec<Vec<Data>> = range.rows().map(|r| r.to_vec()).collect();
    assert_eq!(
        got[0],
        ["zeta", "alpha", "mid", "num", "text"]
            .map(|h| Data::String(h.to_owned()))
            .to_vec(),
        "the header is the first row's keys in order"
    );
    assert_eq!(got[1][0], Data::Bool(true));
    assert_eq!(got[1][1], Data::Empty, "null is an empty cell");
    assert_eq!(
        got[1][2],
        Data::String(r#"{"k":1}"#.to_owned()),
        "an object is its JSON text"
    );
    assert_eq!(got[1][3], Data::Float(2.5));
    assert_eq!(got[1][4], Data::String("x".to_owned()));
    assert_eq!(got[2][0], Data::Bool(false));
    assert_eq!(got[2][1], Data::String("y".to_owned()));
    assert_eq!(
        got[2][2],
        Data::String("[1,2]".to_owned()),
        "an array is its JSON text"
    );
    assert!(
        matches!(got[2][3], Data::Float(v) if v == -3.0) || got[2][3] == Data::Int(-3),
        "{:?}",
        got[2][3]
    );
}

#[actix_web::test]
async fn since_does_not_apply_to_an_xlsx_export() {
    let room = Room::start(&["Alice"]).await;
    let version = room.get(OWNER, "branch=B1").await.body["version"]
        .as_i64()
        .expect("a version");
    let a = room
        .get(
            OWNER,
            &format!("branch=B1&since={version}&purpose=export&format=xlsx"),
        )
        .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.content_type, XLSX, "a workbook, not the unchanged JSON");
    let (_, rows) = a.workbook();
    assert_eq!(rows.len(), 2, "the header and the one user");
    room.finish().await;
}

// ------------------------------------------------------------ the gate and the cap

#[actix_web::test]
async fn an_admin_without_reports_export_is_refused_and_nothing_is_audited() {
    let room = Room::start(&["Alice"]).await;
    let a = room
        .get("ADMIN@*", "branch=B1&purpose=export&format=xlsx")
        .await;
    assert!(a.status.is_client_error(), "{}: {}", a.status, a.text);
    assert_eq!(a.code(), "outOfScope", "{}", a.text);
    assert_eq!(a.content_type, "application/json", "a refusal stays JSON");
    assert!(room.audit().await.is_empty());
    room.finish().await;
}

#[actix_web::test]
async fn fifty_thousand_rows_export_and_one_more_is_too_large_with_no_audit_row() {
    let room = Room::start(&[]).await;
    room.bulk_users(1, 50_000).await;
    let a = room
        .get(OWNER, "branch=B1&purpose=export&format=xlsx")
        .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "exactly the cap exports: {}",
        a.text
    );
    let (_, rows) = a.workbook();
    assert_eq!(rows.len(), 50_001, "the header and 50 000 rows");
    assert_eq!(room.audit().await.len(), 1);

    room.bulk_users(50_001, 1).await;
    let b = room
        .get(OWNER, "branch=B1&purpose=export&format=xlsx")
        .await;
    assert_eq!(b.status, StatusCode::PAYLOAD_TOO_LARGE, "{}", b.text);
    assert_eq!(b.code(), "exportTooLarge", "{}", b.text);
    assert_eq!(b.content_type, "application/json", "a refusal stays JSON");
    assert_eq!(
        room.audit().await.len(),
        1,
        "the refused export is not audited"
    );
    room.finish().await;
}
