mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use actix_web::http::Method;
use chrono::{Duration, Utc};
use pos_error::AppResult;
use rpc_common::{App, Call, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_shift::domain::shift::repositories::shift_balances::{ShiftBalance, ShiftBalances, ShiftId};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const ORIGIN: &str = "http://desk.test";
const MAIN: &str = "MAIN";

#[derive(Clone, Default)]
struct FakeShiftBalances {
    balances: Arc<Mutex<HashMap<ShiftId, Vec<ShiftBalance>>>>,
}

impl FakeShiftBalances {
    fn new() -> Self {
        Self::default()
    }
}

impl ShiftBalances for FakeShiftBalances {
    fn shift_balances<'a>(
        &'a self,
        shift: &'a ShiftId,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftBalance>>> {
        Box::pin(async move {
            let list = self
                .balances
                .lock()
                .unwrap()
                .get(shift)
                .cloned()
                .unwrap_or_default();
            Ok(list)
        })
    }
}

struct TestContext {
    desk: Desk,
    app: App,
    owner_cookie: String,
}

impl TestContext {
    async fn setup() -> Self {
        let desk = Desk::open().await;
        let fake = Arc::new(FakeShiftBalances::new());
        let state = desk.state().with_shift_balances(fake);
        let clock: Arc<dyn xchange::presentation::http::rpc::RpcClock> = desk.clock.clone();
        let bank: Arc<dyn xc_rate::domain::rate::entities::rate_source::RateSource> =
            desk.bank.clone();
        let app = App::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                    .app_data(actix_web::web::Data::from(clock))
                    .app_data(actix_web::web::Data::new(bank)),
            )
            .await,
            |t| t.to_request(),
        );
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let owner_cookie = cookies["OWNER"].clone();
        Self {
            desk,
            app,
            owner_cookie,
        }
    }

    async fn get_admin_shifts(&self, query: &str) -> Value {
        let path = if query.is_empty() {
            "/api/v3/views/admin_shifts".to_string()
        } else {
            format!("/api/v3/views/admin_shifts?{query}")
        };
        let call = Call {
            method: Method::GET,
            host: "desk.test".to_string(),
            path: path.clone(),
            headers: [
                ("origin".to_string(), ORIGIN.to_string()),
                ("x-xchange-request".to_string(), "1".to_string()),
                ("cookie".to_string(), self.owner_cookie.clone()),
            ]
            .into_iter()
            .collect(),
            body: vec![],
        };
        let answer = self.app.send(call).await;
        assert_eq!(answer.status, 200, "GET {path}: {}", answer.text);
        answer.json()["data"].clone()
    }
}

/// R1b: admin_shifts row for a shift with template B
#[actix_web::test]
async fn cs11_r1b_admin_shifts_templated() {
    let ctx = TestContext::setup().await;
    let t0 = Utc::now();

    let shift_id = "S-CLOSED-CS11-R1B";
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, close_note, open_request_id, close_request_id, teller_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, (SELECT id FROM users LIMIT 1))",
    )
    .bind(shift_id)
    .bind(MAIN)
    .bind("CLOSED")
    .bind(t0)
    .bind("owner")
    .bind(Some(t0 + Duration::hours(8)))
    .bind(Some("owner"))
    .bind("note r1b")
    .bind("req-open-cs11-r1b")
    .bind(Some("req-close-cs11-r1b"))
    .execute(&ctx.desk.db.pool)
    .await
    .expect("insert closed shift");

    sqlx::query(
        "UPDATE shifts \
         SET template_id = (SELECT id FROM shift_templates WHERE branch_id = 'MAIN' AND name = 'B'), \
             template_name = 'B', \
             business_date = '2026-11-04'::date \
         WHERE id = $1",
    )
    .bind(shift_id)
    .execute(&ctx.desk.db.pool)
    .await
    .expect("update shift with template");

    let rows = ctx.get_admin_shifts("branch=MAIN").await;
    let row = rows
        .as_array()
        .and_then(|arr| arr.iter().find(|r| r["id"] == json!(shift_id)))
        .unwrap_or_else(|| panic!("shift {shift_id} not found in {rows}"));

    assert_eq!(row["shiftName"], json!("B"), "{row}");
    assert_eq!(row["businessDate"], json!("2026-11-04"), "{row}");
}

/// R1c: admin_shifts row for a shift with no template
#[actix_web::test]
async fn cs11_r1c_admin_shifts_no_template() {
    let ctx = TestContext::setup().await;
    let t0 = Utc::now();

    let shift_id = "S-CLOSED-CS11-R1C";
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, close_note, open_request_id, close_request_id, teller_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, (SELECT id FROM users LIMIT 1))",
    )
    .bind(shift_id)
    .bind(MAIN)
    .bind("CLOSED")
    .bind(t0)
    .bind("owner")
    .bind(Some(t0 + Duration::hours(8)))
    .bind(Some("owner"))
    .bind("note r1c")
    .bind("req-open-cs11-r1c")
    .bind(Some("req-close-cs11-r1c"))
    .execute(&ctx.desk.db.pool)
    .await
    .expect("insert untemplated shift");

    let rows = ctx.get_admin_shifts("branch=MAIN").await;
    let row = rows
        .as_array()
        .and_then(|arr| arr.iter().find(|r| r["id"] == json!(shift_id)))
        .unwrap_or_else(|| panic!("shift {shift_id} not found in {rows}"));

    assert_eq!(row["shiftName"], Value::Null, "{row}");
    assert!(
        row["businessDate"].is_string(),
        "businessDate should be string: {row}"
    );
}
