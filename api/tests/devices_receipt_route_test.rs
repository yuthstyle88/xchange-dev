//! Ticket #403: `GET /devices/receipt` — the 80 mm slip preview the devices page links every
//! print job to (`ตัวอย่างสลิป 80 มม.`) — answers what v2 answered instead of v3's plain 404.
//! Written before the implementation by the lane controller. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! What v2 served, measured in `108-Plaza/xchange-v2-source@4d0bdbb`:
//!
//! - the route: `src/server.mjs:98`
//!   `['GET', /^\/devices\/receipt$/, 'user', (req, p, a) => devices.receipt(new URL(req.url), a)]`
//!   — gate `user`, the same as `/devices` (`:97`): nobody signed in → 303 to `/login`
//!   (`:205`); in v3 that gate is `/devices`' `PAGE_SPECS` row (`who_signed_in: true`).
//! - the handler: `src/devices.mjs:257-271` (`receipt`): `branch` (default `MAIN`) failing
//!   `^[A-Za-z0-9-]{1,70}$` → `400 Invalid branch`; `ref` not an exchange of the branch's
//!   snapshot → `404 ไม่พบรายการ`; otherwise `200 text/html; charset=utf-8`,
//!   `Cache-Control: no-store`, the page below. Staff are on their own branch whatever they
//!   name (`v2_policy_cases.json` `UserRoute` rows of `GET /devices/receipt`).
//! - the page, byte for byte from the handler's template:
//!   `<!doctype html><html lang="th"><meta charset="utf-8"><title>ตัวอย่างสลิป {ref}</title>`
//!   `<style>@page{size:80mm 285mm;margin:4mm}body{box-sizing:border-box;width:72mm;…}</style>`
//!   `<h1>XCHANGE × T.K.</h1><hr><p>สาขา {name}<br>เลขรายการ {ref}<br>ลูกค้า {customerId}<br>`
//!   `{type} {amount} {currency}<br>เรท {rate}<br>ค่าธรรมเนียม {fee} THB<br>`
//!   `{จ่าย|รับ} {thb} THB</p><hr><p>ตัวอย่างก่อนพิมพ์ · พื้นที่พิมพ์ 72 มม. / กระดาษ 80 มม.</p></html>`
//!   Numbers as JavaScript's `toLocaleString()` printed them (`100`, `3,210`, `1,670`, the
//!   rate as stored, `32.1`); `thb` = amount × rate − fee (BUY) / + fee (SELL) — in v3 the
//!   exchange's own `paid` / `received`, which the save computed with `Decimal`.
//!
//! Through the server's whole route table with real sessions (`rpc_common`) and the pages of
//! `tests/fixtures/pages/web`, so `/devices` itself answers its page. Exchanges are saved
//! through the real `SaveTransactionWorkflow` behind the routes.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod shift_setup_common;
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
const MAIN_NAME: &str = "XC-B";
const B9: &str = "B9";
const B9_NAME: &str = "Nine";

const NOT_FOUND: &str = "ไม่พบรายการ";
const INVALID_BRANCH: &str = "Invalid branch";

/// v2's `<style>`, byte for byte (`src/devices.mjs:265`).
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

/// What v2's handler writes from `<!doctype` to the end of the second paragraph.
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

/// One saved exchange: its document number and its customer.
struct Saved {
    reference: String,
    customer: String,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, B9_NAME).await;
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
        for branch in [MAIN, B9] {
            // Re-expected for RE-7 #604 by the ticket author (rates.md §8 migration 6 drops `rates`):
            // the rate is published, per 1 × unit_size, over the current publication; no `rates` row.
            rates_re4e_publish::set_counter_rate(&room.desk.db.pool, branch, "USD", "32.1", "33")
                .await;
            room.open_shift(branch).await;
        }
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

    /// A cash exchange of USD saved through the real workflow behind the routes.
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
            customer,
        }
    }

    /// `GET path` as `who` (`""` = nobody).
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

/// A 200 page that is v2's preview: status, type, no-store, and the template's parts in order.
fn is_v2_preview(answer: &Answer, head: &str, body: &str, what: &str) {
    assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
    assert_eq!(
        answer.header("content-type"),
        Some("text/html; charset=utf-8"),
        "{what}"
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"), "{what}");
    assert!(
        answer.text.starts_with(head),
        "{what}: begins with v2's head\n  want: {head}\n  got:  {:.600}",
        answer.text
    );
    let at = answer.text.find(body).unwrap_or_else(|| {
        panic!(
            "{what}: v2's body, byte for byte\n  want: {body}\n  got:  {}",
            answer.text
        )
    });
    assert!(at >= head.len(), "{what}: the body comes after the head");
    assert!(answer.text.trim_end().ends_with("</html>"), "{what}");
    for marker in ["80mm 285mm", "width:72mm", "14px", "Tahoma", "1px dashed"] {
        assert!(answer.text.contains(marker), "{what}: {marker}");
    }
}

/// A refusal as v2 wrote it: `status`, plain text, exactly `text`.
fn is_plain(answer: &Answer, status: u16, text: &str, what: &str) {
    assert_eq!(answer.status, status, "{what}: {:.300}", answer.text);
    assert_eq!(answer.text, text, "{what}");
    assert!(
        answer
            .header("content-type")
            .is_some_and(|t| t.starts_with("text/plain")),
        "{what}: plain text, got {:?}",
        answer.header("content-type")
    );
}

#[actix_web::test]
async fn a_purchase_is_previewed_as_v2s_80mm_slip() {
    let room = Room::open().await;
    let buy = room.save(MAIN, "BUY", "100", "0").await;
    let answer = room
        .get(
            "OWNER",
            &receipt(&format!("branch=MAIN&ref={}", buy.reference)),
        )
        .await;
    let (head, body) = v2_page(
        &buy.reference,
        MAIN_NAME,
        &format!(
            "ลูกค้า {}<br>BUY 100 USD<br>เรท 32.1<br>ค่าธรรมเนียม 0 THB<br>จ่าย 3,210 THB",
            buy.customer
        ),
    );
    is_v2_preview(&answer, &head, &body, "a BUY of 100 USD at 32.1");
    room.finish().await;
}

#[actix_web::test]
async fn a_sale_says_received_and_adds_the_fee() {
    let room = Room::open().await;
    let sell = room.save(MAIN, "SELL", "50", "20").await;
    let answer = room
        .get("FRONT", &receipt(&format!("ref={}", sell.reference)))
        .await;
    let (head, body) = v2_page(
        &sell.reference,
        MAIN_NAME,
        &format!(
            "ลูกค้า {}<br>SELL 50 USD<br>เรท 33<br>ค่าธรรมเนียม 20 THB<br>รับ 1,670 THB",
            sell.customer
        ),
    );
    is_v2_preview(
        &answer,
        &head,
        &body,
        "a SELL of 50 USD at 33 with a 20 baht fee",
    );
    room.finish().await;
}

/// The gate is `/devices`' (`PAGE_SPECS`, `who_signed_in`): nobody signed in is sent to
/// `/login` exactly as `/devices` sends them; every signed-in caller `/devices` serves is
/// served the preview.
#[actix_web::test]
async fn the_gate_is_the_devices_pages() {
    let room = Room::open().await;
    let buy = room.save(MAIN, "BUY", "100", "0").await;
    let path = receipt(&format!("branch=MAIN&ref={}", buy.reference));

    let page = room.get("", "/devices").await;
    let preview = room.get("", &path).await;
    assert_eq!(page.status, 303, "/devices sends nobody to sign in");
    assert_eq!(preview.status, page.status, "nobody: {:.300}", preview.text);
    assert_eq!(
        page.header("location"),
        Some("http://desk.test/login?next=%2Fdevices")
    );
    assert_eq!(
        preview.header("location"),
        Some(
            format!(
                "http://desk.test/login?next={}",
                path.replace('%', "%25")
                    .replace('/', "%2F")
                    .replace('?', "%3F")
                    .replace('=', "%3D")
                    .replace('&', "%26")
            )
            .as_str()
        )
    );

    let mut served = 0;
    for person in one_of_each_role() {
        let page = room.get(&person.key, "/devices").await;
        assert_eq!(page.status, 200, "/devices serves {}", person.key);
        let preview = room.get(&person.key, &path).await;
        assert_eq!(
            preview.status, 200,
            "{} is served the preview: {:.300}",
            person.key, preview.text
        );
        assert!(preview.text.contains(&buy.reference), "{}", person.key);
        served += 1;
    }
    assert_eq!(served, 10, "the owner and nine staff roles");
    room.finish().await;
}

/// Staff are on their own branch whatever they name; the owner on the one named, `MAIN` when
/// none is.
#[actix_web::test]
async fn staff_preview_their_own_branch_and_the_owner_the_named_one() {
    let room = Room::open().await;
    let main = room.save(MAIN, "BUY", "100", "0").await;
    let nine = room.save(B9, "BUY", "100", "0").await;

    let answer = room
        .get(
            "BACKEND",
            &receipt(&format!("branch=B9&ref={}", main.reference)),
        )
        .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert!(
        answer.text.contains(&format!("สาขา {MAIN_NAME}<br>")),
        "BACKEND of MAIN naming B9 is answered for MAIN: {}",
        answer.text
    );
    let answer = room
        .get(
            "BACKEND",
            &receipt(&format!("branch=B9&ref={}", nine.reference)),
        )
        .await;
    is_plain(
        &answer,
        404,
        NOT_FOUND,
        "B9's exchange is not on MAIN's slip",
    );

    let answer = room
        .get(
            "OWNER",
            &receipt(&format!("branch=B9&ref={}", nine.reference)),
        )
        .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert!(
        answer.text.contains(&format!("สาขา {B9_NAME}<br>")),
        "the owner names B9: {}",
        answer.text
    );
    let answer = room
        .get("OWNER", &receipt(&format!("ref={}", nine.reference)))
        .await;
    is_plain(&answer, 404, NOT_FOUND, "no branch named: MAIN");
    let answer = room
        .get(
            "OWNER",
            &receipt(&format!("branch=&ref={}", main.reference)),
        )
        .await;
    assert_eq!(
        answer.status, 200,
        "an empty branch is MAIN: {:.300}",
        answer.text
    );
    room.finish().await;
}

/// R2, R3: a branch failing the pattern is `400 Invalid branch`; a ref that is not an
/// exchange of the branch — or none at all — is `404 ไม่พบรายการ`.
#[actix_web::test]
async fn a_bad_branch_is_400_and_an_unknown_ref_is_404() {
    let room = Room::open().await;
    let buy = room.save(MAIN, "BUY", "100", "0").await;
    for query in [
        format!("branch=bad%20name&ref={}", buy.reference),
        format!("branch=XC_B&ref={}", buy.reference),
    ] {
        let answer = room.get("OWNER", &receipt(&query)).await;
        is_plain(&answer, 400, INVALID_BRANCH, &query);
    }
    for query in [
        "branch=MAIN&ref=XC-20261006-999999".to_string(),
        "branch=MAIN&ref=".to_string(),
        "branch=MAIN".to_string(),
    ] {
        let answer = room.get("OWNER", &receipt(&query)).await;
        is_plain(&answer, 404, NOT_FOUND, &query);
        let answer = room.get("FRONT", &receipt(&query)).await;
        is_plain(&answer, 404, NOT_FOUND, &format!("{query} as FRONT"));
    }
    room.finish().await;
}
