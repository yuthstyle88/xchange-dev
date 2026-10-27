//! V3 of ticket #5220: `staff_attendance` uses the branch's own timezone (`UTC`), not always Bangkok.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{Duration, Utc};
use chrono_tz::Tz;
use quote_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Permission, RoleGrants};
use xc_kernel::ids::BranchId;
use xchange::application::staff_floor::day::local_day_start;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";

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

#[actix_web::test]
async fn v3_staff_attendance_uses_the_branchs_own_utc_timezone() {
    let db = TestDb::start().await;
    sqlx::query("UPDATE branches SET timezone = 'UTC' WHERE id = 'MAIN'")
        .execute(&db.pool)
        .await
        .expect("set MAIN timezone to UTC");

    let me: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'UTC Teller', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG_1)
    .fetch_one(&db.pool)
    .await
    .expect("insert user");

    let now = Utc::now();
    let start = local_day_start(Tz::UTC, now);
    assert!(start <= now && now - start < Duration::days(1));
    let now_ms = now.timestamp_millis();
    let start_ms = start.timestamp_millis();

    let clock = |position: i32, clock_in: i64, clock_out: Option<i64>| {
        let pool = db.pool.clone();
        let me = me.clone();
        async move {
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
            .bind(&me)
            .bind(data)
            .execute(&pool)
            .await
            .expect("insert attendance");
            id
        }
    };

    let yesterday_open = clock(1, start_ms - 2 * 3_600_000, None).await;
    let yesterday_closed = clock(2, start_ms - 2_000, Some(start_ms - 1_000)).await;
    let today = clock(
        3,
        (start_ms + 1_000).min(now_ms),
        Some((start_ms + 2_000).min(now_ms)),
    )
    .await;

    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
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
        .insert_header((AS, format!("{}:{me}:MAIN", a_reader())))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&atest::read_body(resp).await).expect("json");
    let got: Vec<String> = body["data"]
        .as_array()
        .expect("array")
        .iter()
        .map(|r| r["id"].as_str().expect("id").to_owned())
        .collect();

    assert!(!got.contains(&yesterday_closed));
    assert_eq!(got, vec![today, yesterday_open]);
    db.finish().await;
}
