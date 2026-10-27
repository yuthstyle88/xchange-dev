//! Written before the implementation (ticket #5220). Do not edit, delete or weaken.
//!
//! The `staff_attendance` view answers today's rows and the still-open ones only. "Today" is
//! the branch's calendar day: it starts at `local_day_start(tz, now)`, where `tz` is
//! `branches.timezone`, or `Asia/Bangkok` when that is null or does not parse. The reader
//! asks `CustodyPages::attendance_since(.., since = that start, ..)`, which keeps a row when
//! `clockIn >= since` or when it has no `clockOut`.
//!
//! - D1–D4: `xchange::application::staff_floor::day::local_day_start`, pure.
//! - V1–V2: the view over HTTP (real PG). The rows are placed around the Bangkok day of the
//!   real now, so the tests hold whenever they run: yesterday's rows end 1 s before the day
//!   starts, today's row starts 1 s after it (never after now). The Bangkok day start is
//!   17:00 UTC, never a UTC midnight, so a reader that took the UTC day instead answers a
//!   different set whatever the hour: before 17:00 UTC it drops today's row, after it keeps
//!   yesterday's closed row.
//!
//! V2 ("the branch has no timezone (null)") is pinned with a timezone that does not parse:
//! `branches.timezone` is `NOT NULL DEFAULT 'Asia/Bangkok'` in `api/migrations`, and the view
//! seam's `BranchService::require` (`xc-rate` `branch_repository.rs`) decodes it as a non-null
//! string, so a null never reaches the reader over HTTP. Both cases share the reader's one
//! fallback rule.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing
//! here skips. Each HTTP test works in a schema of its own.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use quote_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Permission, RoleGrants};
use xc_kernel::ids::BranchId;
use xchange::application::staff_floor::day::local_day_start;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

// ------------------------------------------------------------ D: the day start

fn utc(s: &str) -> DateTime<Utc> {
    s.parse().expect("an RFC 3339 UTC instant")
}

/// D1: Bangkok, now 2026-10-27T05:00:00Z (12:00 ICT) → 2026-10-26T17:00:00Z.
#[test]
fn d1_bangkok_midday_starts_at_the_previous_utc_evening() {
    assert_eq!(
        local_day_start(Tz::Asia__Bangkok, utc("2026-10-27T05:00:00Z")),
        utc("2026-10-26T17:00:00Z")
    );
}

/// D2: Bangkok, now 2026-10-26T17:00:00Z (00:00 ICT) → itself: the start is inclusive.
#[test]
fn d2_bangkok_exactly_at_midnight_is_its_own_day_start() {
    assert_eq!(
        local_day_start(Tz::Asia__Bangkok, utc("2026-10-26T17:00:00Z")),
        utc("2026-10-26T17:00:00Z")
    );
}

/// D3: Bangkok, now 2026-10-26T16:59:59Z (23:59:59 ICT) → 2026-10-25T17:00:00Z.
#[test]
fn d3_bangkok_one_second_before_midnight_is_still_the_previous_day() {
    assert_eq!(
        local_day_start(Tz::Asia__Bangkok, utc("2026-10-26T16:59:59Z")),
        utc("2026-10-25T17:00:00Z")
    );
}

/// D4: UTC, now 2026-10-27T05:00:00Z → 2026-10-27T00:00:00Z.
#[test]
fn d4_utc_starts_at_utc_midnight() {
    assert_eq!(
        local_day_start(Tz::UTC, utc("2026-10-27T05:00:00Z")),
        utc("2026-10-27T00:00:00Z")
    );
}

// ------------------------------------------------------------ the signed-in person

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";

/// A staff session from the header `x-test-as: <V2ROLE>:<user id>:<branch>`, mapped as
/// production maps it.
struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let mut p = spec.splitn(3, ':');
            let staff_user = match (p.next(), p.next(), p.next()) {
                (Some(role), Some(id), Some(branch)) => test_callers::v2_staff_roles(role)
                    .map(|roles| test_callers::staff(id, roles, BranchId::new(branch))),
                _ => None,
            };
            Callers {
                user: staff_user.clone(),
                staff_user,
            }
        })
    }
}

/// A staff role whose session holds `AttendanceClock` (reads the view), read from the role
/// table, never a role literal.
fn a_reader() -> &'static str {
    test_callers::V2_STAFF
        .into_iter()
        .map(|(r, _)| r)
        .find(|r| {
            let roles = test_callers::v2_staff_roles(r).expect("a staff role");
            test_callers::staff("x", roles, BranchId::new("MAIN"))
                .roles
                .iter()
                .any(|s| RoleGrants::seed(s.role).contains(&Permission::AttendanceClock))
        })
        .expect("a staff role that reads staff_attendance")
}

// ------------------------------------------------------------ the floor

struct Floor {
    db: TestDb,
}

impl Floor {
    async fn start() -> Self {
        Self {
            db: TestDb::start().await,
        }
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

    /// `staff_attendance` of MAIN as `role` signed in as `id`; answers `data`.
    async fn attendance(&self, role: &str, id: &str) -> Value {
        let state = AppState::new(Arc::new(common::settings(&[])), self.db.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let req = atest::TestRequest::get()
            .uri("/api/v3/views/staff_attendance")
            .insert_header(("host", HOST))
            .insert_header(("x-xchange-request", "1"))
            .insert_header((AS, format!("{role}:{id}:MAIN")))
            .to_request();
        let resp = atest::call_service(&app, req).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        assert_eq!(status, StatusCode::OK, "staff_attendance: {body}");
        body["data"].clone()
    }

    /// One attendance row of `who` at MAIN, at `position` (higher = newer); times in epoch ms.
    async fn clock(
        &self,
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
            "INSERT INTO attendance (id, branch_id, position, employee_id, data) \
             VALUES ($1, 'MAIN', $2, $3, $4)",
        )
        .bind(&id)
        .bind(position)
        .bind(who)
        .bind(data)
        .execute(&self.db.pool)
        .await
        .expect("insert attendance");
        id
    }

    async fn exec(&self, sql: &'static str) {
        sqlx::query(sql)
            .execute(&self.db.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

/// The start of `now`'s Bangkok calendar day as a UTC instant, computed here from
/// `chrono_tz` (not from the code under test).
fn bangkok_day_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let midnight = now
        .with_timezone(&Tz::Asia__Bangkok)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight");
    Tz::Asia__Bangkok
        .from_local_datetime(&midnight)
        .single()
        .expect("Bangkok has no DST")
        .with_timezone(&Utc)
}

fn instant(v: &Value, what: &str) -> DateTime<Utc> {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a time string, got {v}"));
    DateTime::parse_from_rfc3339(s)
        .unwrap_or_else(|e| panic!("{what} {s:?}: {e}"))
        .with_timezone(&Utc)
}

fn ids(rows: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap_or_else(|| panic!("data is an array, got {rows}"))
        .iter()
        .map(|r| r["id"].as_str().expect("a row id").to_owned())
        .collect()
}

/// V1's rows, all of `me` at MAIN, around the Bangkok day of the real now:
/// - position 1: yesterday (ICT), clocked in 2 h before the day starts, still open;
/// - position 2: yesterday (ICT), clocked in 2 s and out 1 s before the day starts;
/// - position 3: today (ICT), clocked in 1 s and out 2 s after the day starts (never after now).
struct Rows {
    yesterday_open: String,
    yesterday_closed: String,
    today: String,
    today_in: i64,
    today_out: i64,
}

async fn seed(f: &Floor, me: &str) -> Rows {
    let now = Utc::now();
    let start = bangkok_day_start(now);
    let now_ms = now.timestamp_millis();
    let start_ms = start.timestamp_millis();
    assert!(start <= now && now - start < Duration::days(1));

    let today_in = (start_ms + 1_000).min(now_ms);
    let today_out = (start_ms + 2_000).min(now_ms);
    let yesterday_open = f.clock(1, me, start_ms - 2 * 3_600_000, None).await;
    let yesterday_closed = f
        .clock(2, me, start_ms - 2_000, Some(start_ms - 1_000))
        .await;
    let today = f.clock(3, me, today_in, Some(today_out)).await;
    Rows {
        yesterday_open,
        yesterday_closed,
        today,
        today_in,
        today_out,
    }
}

fn assert_today_and_still_open(rows: &Value, seeded: &Rows, what: &str) {
    let got = ids(rows);
    assert!(
        !got.contains(&seeded.yesterday_closed),
        "{what}: yesterday's closed row is not today's: {got:?}"
    );
    assert_eq!(
        got,
        [seeded.today.clone(), seeded.yesterday_open.clone()],
        "{what}: today's row and yesterday's still-open row, newest first",
    );
    let today = &rows[0];
    assert_eq!(
        instant(&today["in"], "in").timestamp_millis(),
        seeded.today_in,
        "{what}: today's in"
    );
    assert_eq!(
        instant(&today["out"], "out").timestamp_millis(),
        seeded.today_out,
        "{what}: today's out"
    );
    assert!(
        rows[1].get("out").is_none_or(Value::is_null),
        "{what}: yesterday's open row is still clocked in"
    );
}

// ------------------------------------------------------------ V: the view

/// V1: the branch's timezone is Asia/Bangkok. Yesterday's closed row is not answered;
/// today's row and yesterday's still-open row are, newest first.
#[actix_web::test]
async fn v1_staff_attendance_answers_todays_rows_and_still_open_ones_in_the_branch_day() {
    let f = Floor::start().await;
    f.exec("UPDATE branches SET timezone = 'Asia/Bangkok' WHERE id = 'MAIN'")
        .await;
    let me = f.person("Me Myself").await;
    let seeded = seed(&f, &me).await;

    let rows = f.attendance(a_reader(), &me).await;
    assert_today_and_still_open(&rows, &seeded, "V1");
    assert_eq!(
        rows[0]["name"],
        json!("Me Myself"),
        "V1: the row mapping stays"
    );
    f.finish().await;
}

/// V2: the branch has no usable timezone (it does not parse, read as null): the Bangkok day
/// is used.
#[actix_web::test]
async fn v2_a_branch_timezone_that_does_not_parse_reads_the_bangkok_day() {
    let f = Floor::start().await;
    f.exec("UPDATE branches SET timezone = 'Not/A_Zone' WHERE id = 'MAIN'")
        .await;
    let me = f.person("Me Myself").await;
    let seeded = seed(&f, &me).await;

    let rows = f.attendance(a_reader(), &me).await;
    assert_today_and_still_open(&rows, &seeded, "V2 unparseable timezone");
    f.finish().await;
}
