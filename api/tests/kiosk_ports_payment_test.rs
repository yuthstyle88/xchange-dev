//! Written before the implementation (row PM-K, design `docs/design/xchange-v3.md` §5.5.2).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The real `PgKioskPorts` reads the branch's payment settings — the ones the owner saves at
//! `POST /api/payment-methods` (row PM-API) — so the kiosk's `quote` and `confirm` on
//! `POST /api/kiosk` refuse a channel the owner switched off for that direction, in that branch
//! only. `PgKioskPorts` builds its `PaymentSettingsService` from the pool it already holds:
//! `PgKioskPorts::new` keeps its signature and neither of its callers is edited.
//!
//! Everything goes through the server's whole route table with real sessions
//! (`kiosk_http_common::Counter`: `MAIN` with the dollar at 32.1 / 32.6 and an open shift).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::{Counter, as_who};
use rpc_common::Call;
use serde_json::{Value, json};

const UNAVAILABLE: &str = "Payment method unavailable. Please contact staff.";

/// v2's defaults (`payment-settings.mjs:3` of the 10-06 bundle): every channel on, BUY on Cash
/// and Bank Transfer only.
fn defaults() -> Value {
    json!([
        { "id": "Cash", "enabled": true, "buy": true, "sell": true, "account": "", "instruction": "" },
        { "id": "Credit Card", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "Alipay", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "PromptPay", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "Bank Transfer", "enabled": true, "buy": true, "sell": true, "account": "",
          "instruction": "If transfer, call agent · หากโอนเงิน กรุณาติดต่อพนักงาน" },
    ])
}

fn switched(id: &str, enabled: bool) -> Value {
    let mut rows = defaults();
    for row in rows.as_array_mut().expect("five rows") {
        if row["id"] == json!(id) {
            row["enabled"] = json!(enabled);
        }
    }
    rows
}

/// The owner saves `methods` for `branch` at the admin page's door.
async fn owner_saves(counter: &Counter, branch: &str, version: i64, methods: Value) {
    let call = Call::action(
        &counter.origin,
        "/api/payment-methods",
        "save",
        json!({ "branchId": branch, "version": version, "methods": methods }),
    );
    let answer = counter
        .app
        .send(as_who(call, &counter.cookies, "owner"))
        .await;
    assert_eq!(
        answer.status, 200,
        "the owner saves the payment settings of {branch}: {:.300}",
        answer.text
    );
    assert_eq!(answer.json()["version"], json!(version + 1));
}

fn sell(method: &str, branch: Option<&str>) -> Value {
    let mut payload =
        json!({ "type": "SELL", "currency": "USD", "amount": 1000, "paymentMethod": method });
    if let Some(branch) = branch {
        payload["branchId"] = json!(branch);
    }
    payload
}

async fn quoted(counter: &Counter, id: &str, payload: Value) -> String {
    counter.hands.ids.push(id);
    let answer = counter.ask(None, "quote", payload).await;
    assert_eq!(answer.status, 200, "quote {id}: {:.300}", answer.text);
    assert_eq!(answer.json()["id"], json!(id));
    id.to_string()
}

fn later(counter: &Counter, seconds: i64) {
    counter
        .desk
        .clock
        .set(counter.now() + chrono::Duration::seconds(seconds));
}

#[actix_web::test]
async fn a_quote_on_a_channel_the_owner_switched_off_is_refused() {
    let counter = Counter::open(&[]).await;
    owner_saves(&counter, "MAIN", 0, switched("Credit Card", false)).await;

    counter
        .ask(None, "quote", sell("Credit Card", None))
        .await
        .assert_refused(400, UNAVAILABLE, "SELL by card, cards off");
    quoted(
        &counter,
        "00000000-0000-4000-8000-0000000d0001",
        sell("PromptPay", None),
    )
    .await;
    counter.finish().await;
}

#[actix_web::test]
async fn a_confirm_after_the_switch_is_refused_and_passes_once_switched_back() {
    let counter = Counter::open(&[]).await;
    let id = quoted(
        &counter,
        "00000000-0000-4000-8000-0000000d0002",
        sell("Alipay", None),
    )
    .await;
    owner_saves(&counter, "MAIN", 0, switched("Alipay", false)).await;
    later(&counter, 4);
    counter
        .ask(None, "confirm", json!({ "id": id }))
        .await
        .assert_refused(400, UNAVAILABLE, "confirm after Alipay was switched off");

    owner_saves(&counter, "MAIN", 1, defaults()).await;
    let confirmed = counter.ask(None, "confirm", json!({ "id": id })).await;
    assert_eq!(confirmed.status, 200, "{}", confirmed.text);
    assert_eq!(confirmed.json()["status"], json!("WAITING"));
    counter.finish().await;
}

/// "มันต้องรองรับหลาย สาขา": the switch of `MAIN` does not reach another branch's kiosk.
#[actix_web::test]
async fn each_branch_kiosk_reads_its_own_switch() {
    let counter = Counter::open(&[]).await;
    let made = counter
        .owner_rpc(
            "adminCreateBranch",
            json!({ "requestId": "00000000-0000-4000-8000-0000000d0101", "name": "Airport" }),
        )
        .await;
    let airport = made["branchId"]
        .as_str()
        .expect("the new branch")
        .to_string();
    // LG-4 (#929, 1:1): the airport's opening below leaves its ledger SAFE (design §4.4).
    snapshot_common::lg4_fund_safe(&counter.desk.db.pool, &airport).await;
    // #756 (RE-4b, rates.md D-3, §4.9): the kiosk's counter rate is the current publication.
    // Re-expected for RE-7 #604 by the ticket author: no `adminSaveRate` first (design §9 removes
    // the action); the publication alone sets the airport's dollar.
    kiosk_http_common::rates_re4_common::publish(
        &counter.desk.db.pool,
        &airport,
        &[("USD", "32", "32.5")],
    )
    .await;
    counter
        .owner_rpc(
            "adminOpenShift",
            json!({ "requestId": "00000000-0000-4000-8000-0000000d0103", "branchId": airport,
                    "opening": { "THB": 100000, "USD": 2000 } }),
        )
        .await;

    owner_saves(&counter, "MAIN", 0, switched("Credit Card", false)).await;
    quoted(
        &counter,
        "00000000-0000-4000-8000-0000000d0104",
        sell("Credit Card", Some(&airport)),
    )
    .await;
    counter
        .ask(None, "quote", sell("Credit Card", Some("MAIN")))
        .await
        .assert_refused(400, UNAVAILABLE, "MAIN's cards are off");

    owner_saves(&counter, &airport, 0, switched("PromptPay", false)).await;
    counter
        .ask(None, "quote", sell("PromptPay", Some(&airport)))
        .await
        .assert_refused(400, UNAVAILABLE, "the airport's PromptPay is off");
    quoted(
        &counter,
        "00000000-0000-4000-8000-0000000d0105",
        sell("PromptPay", Some("MAIN")),
    )
    .await;
    counter.finish().await;
}
