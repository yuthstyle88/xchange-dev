//! Ticket #445: route-level test for GET /devices/receipt query failure.
//!
//! When a database query fails during receipt generation (e.g. `transactions` table
//! error), the route answers 500 Internal Server Error with Content-Type
//! `application/json; charset=utf-8` and body exactly `{"error":"เซิร์ฟเวอร์ขัดข้อง"}`.
//!
//! Needs TEST_DATABASE_URL; nothing here skips.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod snapshot_common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::json;
use xc_exchange::application::customer::service::CustomerService;
use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::AppState;

const ORIGIN: &str = "http://desk.test";
const RECEIPT: &str = "/devices/receipt";
const MAIN: &str = "MAIN";

const SERVER_ERROR_BODY: &str = "{\"error\":\"เซิร์ฟเวอร์ขัดข้อง\"}";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", next())
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

struct Saved {
    reference: String,
    _customer: String,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages/web");
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_WEB__DIR",
                web.to_str().expect("a UTF-8 path"),
            )])),
            desk.db.pool.clone(),
        );
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
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        let room = Room {
            desk,
            app,
            cookies,
            t0,
        };
        sqlx::query(
            "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
             VALUES ($1, 'USD', 32.1, 33, now(), 'test', $2)",
        )
        .bind(MAIN)
        .bind(request_id())
        .execute(&room.desk.db.pool)
        .await
        .expect("insert a counter rate");
        // #759 (RE-4e; rates.md D-3, §4.9 R4): the counter rate is the current publication.
        rates_re4e_publish::publish_newest_rates(&room.desk.db.pool, MAIN).await;
        room.open_shift(MAIN).await;
        room
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    async fn open_shift(&self, branch: &str) {
        self.tick();
        let answer = self
            .app
            .send(
                Call::action(
                    ORIGIN,
                    "/api/rpc",
                    "adminOpenShift",
                    json!({ "branchId": branch, "requestId": request_id(),
                            "opening": { "THB": 100000, "USD": 1000 } }),
                )
                .cookie(&self.cookies["OWNER"]),
            )
            .await;
        assert_eq!(answer.status, 200, "open a shift: {:.300}", answer.text);
    }

    async fn save(&self, branch: &str, side: &str, amount: &str, fee: &str) -> Saved {
        let t = self.tick();
        let here = BranchId::new(branch);
        let person = ReserveCustomer {
            name: "Somchai Jaidee".to_owned(),
            passport_no: "AA1234567".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: "LIVE".to_owned(),
        };
        let customer = CustomerService::new(self.desk.db.branch_tx())
            .reserve(&here, person.clone(), t)
            .await
            .expect("reserve the customer")
            .customer_id;
        let passport = self
            .desk
            .files
            .upload(&here, "owner", "image/jpeg", JPEG, t)
            .await
            .expect("upload a passport");
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id: customer.clone(),
            side: side.to_owned(),
            currency: "USD".to_owned(),
            amount: amount.to_owned(),
            rate: if side == "BUY" { "32.1" } else { "33" }.to_owned(),
            fee: fee.to_owned(),
            bank_fee: "0".to_owned(),
            method: "Cash".to_owned(),
            payment_ref: String::new(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: "2026-10-06".to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport),
            slip: None,
            reference_rate: None,
        };
        let reference = self
            .desk
            .workflow
            .save(&here, cmd, "owner", t)
            .await
            .unwrap_or_else(|e| panic!("save a {side} in {branch}: {e:?}"))
            .document_ref;
        Saved {
            reference,
            _customer: customer,
        }
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
            .without("x-xchange-request")
            .without("content-type")
            .without("origin");
        call.method = Method::GET;
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookies[who])
        };
        self.app.send(call).await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn receipt(query: &str) -> String {
    format!("{RECEIPT}?{query}")
}

#[tokio::test]
async fn query_error_answers_500_with_thai_error_json() {
    let room = Room::open().await;
    let saved = room.save(MAIN, "BUY", "100", "0").await;

    // Before renaming the table, the preview succeeds with 200.
    let path = receipt(&format!("branch={MAIN}&ref={}", saved.reference));
    let before = room.get("OWNER", &path).await;
    assert_eq!(before.status, 200);

    // Make the transactions query fail by renaming the table inside the test's database.
    sqlx::query("ALTER TABLE transactions RENAME TO transactions_hidden")
        .execute(&room.desk.db.pool)
        .await
        .expect("rename transactions table");

    // Owner request answers 500 with application/json; charset=utf-8 and exact Thai error body.
    let answer = room.get("OWNER", &path).await;
    assert_eq!(answer.status, 500);
    assert_eq!(
        answer.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(answer.text, SERVER_ERROR_BODY);

    // Staff request on their own branch also answers 500 with exact Thai error body.
    let staff_path = receipt(&format!("ref={}", saved.reference));
    let staff_answer = room.get("FRONT", &staff_path).await;
    assert_eq!(staff_answer.status, 500);
    assert_eq!(
        staff_answer.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(staff_answer.text, SERVER_ERROR_BODY);

    room.finish().await;
}
