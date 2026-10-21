//! Ticket #3843 — `admin_customers` search (`q`) combined with period (`from`/`to`) filter.
//!
//! When `q` matches a customer's name (or id) and `from`/`to` is also supplied, a customer with no
//! completed transaction in the period must NOT be returned.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
#[macro_use]
mod rpc_common;
mod shift_holder_common;
mod shift_setup_common;
mod snapshot_common;

use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::Duration;
use rpc_common::{App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kernel::ids::BranchId;

const ORIGIN: &str = "http://desk.test";
const MAIN: &str = "MAIN";
const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn reserve_cmd(name: &str) -> ReserveCustomer {
    ReserveCustomer {
        name: name.to_owned(),
        passport_no: format!("AA{:07}", next()),
        nationality: "THA".to_owned(),
        birth_date: String::new(),
        passport_expiry: String::new(),
        mode: "LIVE".to_owned(),
    }
}

async fn get_ids(app: &App, cookies: &Cookies, query: &str) -> Vec<String> {
    let path = format!("/api/v3/views/admin_customers?{query}");
    let mut call = Call::post(ORIGIN, &path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin")
        .cookie(&cookies["OWNER"]);
    call.method = Method::GET;
    let answer = app.send(call).await;
    assert_eq!(answer.status, 200, "GET {path}: {:.400}", answer.text);
    let body = answer.json();
    assert_eq!(body["changed"], json!(true), "GET {path}: {body}");
    body["data"]
        .as_array()
        .expect("data array")
        .iter()
        .map(|r: &Value| r["id"].as_str().expect("string id").to_owned())
        .collect()
}

#[actix_web::test]
async fn customer_matching_q_without_completed_tx_in_period_is_not_returned() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;
    let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
    desk.clock.set(t0);

    rates_re4e_publish::set_counter_rate(&desk.db.pool, MAIN, "USD", "32.1", "33").await;
    let t_shift = t0 + Duration::seconds(next() as i64);
    desk.clock.set(t_shift);
    shift_setup_common::open_shift_by_count(
        &desk.db.pool,
        MAIN,
        json!({ "THB": 100000, "USD": 1000 }),
        t_shift,
    )
    .await;

    let here = BranchId::new(MAIN);
    let cust_svc = CustomerService::new(desk.db.branch_tx());

    // Customer A: name matches `q` ("PeriodTarget"), has NO completed transaction in the period.
    let t_a = t0 + Duration::seconds(next() as i64);
    desk.clock.set(t_a);
    let cust_a = cust_svc
        .reserve(&here, reserve_cmd("Alice PeriodTarget"), t_a)
        .await
        .expect("reserve Customer A")
        .customer_id;

    // Customer B: name also matches `q` ("PeriodTarget"), HAS a completed transaction in the period.
    let t_b = t0 + Duration::seconds(next() as i64);
    desk.clock.set(t_b);
    let cmd_b = reserve_cmd("Bob PeriodTarget");
    let cust_b = cust_svc
        .reserve(&here, cmd_b.clone(), t_b)
        .await
        .expect("reserve Customer B")
        .customer_id;

    let t_save = t0 + Duration::seconds(next() as i64);
    desk.clock.set(t_save);
    let saver = shift_holder_common::teller_of_open_shift(&desk.db.pool, MAIN).await;
    let passport = desk
        .files
        .upload(&here, &saver, "image/jpeg", JPEG, t_save)
        .await
        .expect("upload passport");
    desk.workflow
        .save(
            &here,
            SaveTransaction {
                customer: cmd_b,
                submission_id: format!("submission-MAIN-{:012}", next()),
                customer_id: cust_b.clone(),
                side: "BUY".to_owned(),
                currency: "USD".to_owned(),
                amount: "100".to_owned(),
                rate: "32.1".to_owned(),
                fee: "0".to_owned(),
                bank_fee: "0".to_owned(),
                method: "Cash".to_owned(),
                payment_ref: String::new(),
                account: "Cash drawer 1".to_owned(),
                staff: "EMP-3843".to_owned(),
                booking: "WALK-IN".to_owned(),
                notes: String::new(),
                value_date: "2026-10-06".to_owned(),
                paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
                payment_confirmed: true,
                identity_confirmed: true,
                passport: Some(passport),
                slip: None,
                reference_rate: None,
            },
            &saver,
            t_save,
        )
        .await
        .expect("save transaction for Customer B");

    let ids = get_ids(
        &app,
        &cookies,
        "branch=MAIN&limit=100&q=PeriodTarget&from=2026-10-06&to=2026-10-06",
    )
    .await;

    assert!(
        !ids.contains(&cust_a),
        "Customer A matched by name without a completed transaction in the period must not be returned (got: {ids:?})"
    );
    assert_eq!(ids, vec![cust_b]);

    desk.finish().await;
}
