//! Ticket #399 (V1-kiosk-currencies): extra tests for currencies without KBank notes.
//!
//! §9 edge case: a currency KBank has no note for with a shop rate (AFN buy 0.4500)
//! → one row, label AFN, shopBuy "0.45000", the bank values and differences null.
//!
//! Mutant R2: dropping shop figures on the no-note arm (shop_buy: None) must turn this test RED.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const VIEW: &str = "kiosk_currencies";
const MAIN: &str = "MAIN";

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn path(query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{VIEW}")
    } else {
        format!("/api/v3/views/{VIEW}?{query}")
    }
}

fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);
        Room {
            desk,
            app,
            cookies,
            t0,
        }
    }

    fn tick(&self) -> DateTime<Utc> {
        let n = next() as i64;
        let t = self.t0 + Duration::seconds(n) + Duration::microseconds(123_456);
        self.desk.clock.set(t);
        t
    }

    fn cookie(&self, who: &str) -> String {
        self.cookies[who].clone()
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    async fn view(&self, who: &str, query: &str) -> Value {
        let answer = self.get(who, &path(query)).await;
        assert_eq!(answer.status, 200);
        answer.json()
    }

    async fn data(&self, who: &str, query: &str) -> Value {
        let body = self.view(who, query).await;
        assert_eq!(body["changed"], json!(true));
        body.get("data").cloned().expect("data in body")
    }

    async fn set_rate(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        let at = self.tick();
        sqlx::query(
            "INSERT INTO rates (branch_id, code, buy, sell, partner_name, partner_cost, \
             updated_at, updated_by, request_id) \
             VALUES ($1, $2, $3::numeric, $4::numeric, '', NULL, $5, 'test', $6)",
        )
        .bind(branch)
        .bind(code)
        .bind(buy)
        .bind(sell)
        .bind(at)
        .bind(format!("req-{}", next()))
        .execute(&self.desk.db.pool)
        .await
        .expect("insert a counter rate");
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn currency<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["currencies"]
        .as_array()
        .expect("currencies array")
        .iter()
        .find(|c| c["code"] == json!(code))
        .expect("currency found")
}

#[actix_web::test]
async fn currency_with_no_kbank_note_shows_shop_figures_with_null_bank_figures() {
    let room = Room::open().await;
    room.set_rate(MAIN, "AFN", "0.4500", "0.5000").await;
    let data = room.data("OWNER", "branch=MAIN").await;
    let afn = currency(&data, "AFN");
    assert_eq!(
        afn["kbank"],
        json!([
            {
                "label": "AFN",
                "bankBuy": null,
                "shopBuy": "0.45000",
                "buyDifference": null,
                "bankSell": null,
                "shopSell": "0.50000",
                "sellDifference": null
            }
        ]),
        "AFN kbank comparison with shop rates and null bank values"
    );
    room.finish().await;
}
