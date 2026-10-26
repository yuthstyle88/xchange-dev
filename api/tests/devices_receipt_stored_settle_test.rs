//! Up-front test for CR-c1 (#5119): `GET /devices/receipt` shows the stored `settle_thb` from
//! `transaction_lines` rather than recomputing `quantity × rate ÷ unit_size ∓ fee`.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
#[macro_use]
mod rpc_common;
mod shift_holder_common;
mod shift_setup_common;
mod snapshot_common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::json;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::AppState;

const ORIGIN: &str = "http://desk.test";
const RECEIPT: &str = "/devices/receipt";
const MAIN: &str = "MAIN";
const MAIN_NAME: &str = "XC-B";

const STYLE: &str = "<style>@page{size:80mm 285mm;margin:4mm}body{box-sizing:border-box;width:72mm;\
margin:0 auto;overflow-wrap:anywhere;font:14px/1.7 Tahoma,sans-serif}h1{text-align:center;\
font-size:20px}hr{border:0;border-top:1px dashed #333}</style>";
const FOOTER: &str = "<hr><p>ตัวอย่างก่อนพิมพ์ · พื้นที่พิมพ์ 72 มม. / กระดาษ 80 มม.</p>";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[allow(dead_code)]
fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", next())
}

fn v2_page(reference: &str, branch_name: &str, lines: &str) -> (String, String) {
    let head = format!(
        "<!doctype html><html lang=\"th\"><meta charset=\"utf-8\"><title>ตัวอย่างสลิป {reference}</title>{STYLE}"
    );
    let body = format!(
        "<h1>XCHANGE × T.K.</h1><hr><p>สาขา {branch_name}<br>เลขรายการ {reference}<br>{lines}</p>{FOOTER}"
    );
    (head, body)
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

struct Saved {
    reference: String,
    customer: String,
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
        rates_re4e_publish::set_counter_rate(&room.desk.db.pool, MAIN, "USD", "32.1", "33").await;
        room.open_shift(MAIN).await;
        room
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    async fn open_shift(&self, branch: &str) {
        let t = self.tick();
        shift_setup_common::open_shift_by_count(
            &self.desk.db.pool,
            branch,
            json!({ "THB": 100000, "USD": 1000 }),
            t,
        )
        .await;
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
        let saver = shift_holder_common::teller_of_open_shift(&self.desk.db.pool, branch).await;
        let passport = self
            .desk
            .files
            .upload(&here, &saver, "image/jpeg", JPEG, t)
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
            .save(&here, cmd, &saver, t)
            .await
            .unwrap_or_else(|e| panic!("save a {side} in {branch}: {e:?}"))
            .document_ref;
        Saved {
            reference,
            customer,
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

/// R4: `/devices/receipt` for a completed transaction whose `transaction_lines.settle_thb` was set
/// to 32.25 (`UPDATE` in the fixture) while quantity × rate is 32.10 shows `32.25`, not `32.1`.
#[actix_web::test]
async fn r4_devices_receipt_shows_stored_settle_thb() {
    let room = Room::open().await;
    let buy = room.save(MAIN, "BUY", "1", "0").await;

    sqlx::query(
        "UPDATE transaction_lines tl \
         SET settle_thb = 32.25 \
         FROM transactions t \
         WHERE tl.transaction_id = t.id AND t.branch_id = $1 AND t.document_ref = $2",
    )
    .bind(MAIN)
    .bind(&buy.reference)
    .execute(&room.desk.db.pool)
    .await
    .expect("update settle_thb to 32.25");

    let answer = room
        .get(
            "OWNER",
            &receipt(&format!("branch={MAIN}&ref={}", buy.reference)),
        )
        .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    let (head, body) = v2_page(
        &buy.reference,
        MAIN_NAME,
        &format!(
            "ลูกค้า {}<br>BUY 1 USD<br>เรท 32.1<br>ค่าธรรมเนียม 0 THB<br>จ่าย 32.25 THB",
            buy.customer
        ),
    );
    assert!(answer.text.starts_with(&head), "{}", answer.text);
    assert!(
        answer.text.contains(&body),
        "expected stored settle_thb 32.25 in body\n  want: {body}\n  got:  {}",
        answer.text
    );
    room.finish().await;
}
