//! Ticket #380 — Configured printer models are ignored: no route builds DeviceService
//! with devices.printerModels.
//!
//! Two construction sites lacked `.with_printer_models(printer_models)`:
//! - `api/src/presentation/http/devices.rs:43`
//! - `api/src/presentation/http/rpc.rs:300`
//!
//! Written before the fix. Must fail on main, and must go red if either construction site is reverted.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
#[macro_use]
mod rpc_common;
mod shift_holder_common;
mod snapshot_common;

use std::sync::Arc;

use chrono::Utc;
use rpc_common::{App, Call, Cookies, Desk, MAIN, RPC, one_of_each_role, sign_in};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use xc_kernel::ids::BranchId;
use xchange::application::compat::envelope::RpcRoute;
use xchange::bootstrap::AppState;
use xchange::infrastructure::devices::printer_models::{DrawerPulse, PrinterModel};
use xchange::presentation::http::rpc::registered_actions;

const ORIGIN: &str = "http://desk.test";

const CUSTOM_MODEL: &str = "CUSTOM_POS_80";
const CUSTOM_PULSE: DrawerPulse = DrawerPulse {
    pin: 0,
    on: 30,
    off: 200,
};
/// `ESC @` + `ESC p 0 30 200` (`[0x1B, 0x40, 0x1B, 0x70, 0, 30, 200]`), base64.
const CUSTOM_KICK_BASE64: &str = "G0AbcAAeyA==";

const AGENT_ID: &str = "a0000000-0000-4000-8000-000000000001";
const AGENT_TOKEN: &str = "xcda_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn custom_model() -> PrinterModel {
    PrinterModel {
        id: CUSTOM_MODEL.to_owned(),
        name: "Custom POS-80".to_owned(),
        paper_mm: 80,
        drawer_pulse: Some(CUSTOM_PULSE),
    }
}

async fn setup_app_and_state(desk: &Desk) -> (App, AppState, Cookies) {
    let mut settings = (*desk.state().settings).clone();
    settings.devices.printer_models = vec![custom_model()];
    let state = AppState::new(Arc::new(settings), desk.db.pool.clone());

    let clock: Arc<dyn xchange::presentation::http::rpc::RpcClock> = desk.clock.clone();
    let bank: Arc<dyn xc_rate::domain::rate::entities::rate_source::RateSource> = desk.bank.clone();
    let state_for_app = state.clone();

    let app = App::new(
        actix_web::test::init_service(
            actix_web::App::new()
                .configure(move |cfg| xchange::bootstrap::configure(cfg, state_for_app.clone()))
                .app_data(actix_web::web::Data::from(clock))
                .app_data(actix_web::web::Data::new(bank)),
        )
        .await,
        |t| t.to_request(),
    );

    // OR-4b (#767, design §4.2 C2): ExchangeTransact is TELLER's, so FRONT (TELLER under B1) is
    // employed and signed in beside the owner, and saves
    let people: Vec<_> = one_of_each_role()
        .into_iter()
        .filter(|p| p.key == "OWNER" || p.key == "FRONT")
        .collect();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;

    (app, state, cookies)
}

async fn setup_active_agent(desk: &Desk) {
    sqlx::query(
        "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, token_hash, \
         paired_at, last_seen_at, agent_version, agent_build, agent_os, agent_host, \
         drawer_transport, drawer_target) VALUES ($1::uuid, $2, 'Counter 1', now(), 'owner', $3, now(), now(), \
         '1.0.0', 'abc1234', 'macos-14.6-arm64', 'counter-1.local', 'CUPS', 'POS80_')",
    )
    .bind(AGENT_ID)
    .bind(MAIN)
    .bind(sha256_hex(AGENT_TOKEN))
    .execute(&desk.db.pool)
    .await
    .expect("insert active agent");
}

async fn setup_cash_save_prerequisites(desk: &Desk) -> String {
    // Re-expected for RE-7 #604 by the ticket author (rates.md §8 migration 6 drops `rates`):
    // the rate is published, per 1 × unit_size, over the current publication; no `rates` row.
    rates_re4e_publish::set_counter_rate(&desk.db.pool, MAIN, "USD", "32.1", "33.0").await;

    // CA-10 (#626, ruling A), setup only: the shift is the saver's own, so its teller is the user FRONT's
    // employee id resolves to (`teller_of_actor_in`), not a user named "FRONT".
    let front = one_of_each_role()
        .into_iter()
        .find(|p| p.key == "FRONT")
        .expect("FRONT");
    let front_user_id = shift_holder_common::user_of(&desk.db.pool, &front.id).await;
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ('shift-1', $1, 'OPEN', now(), 'FRONT', 'open-req-1', $2::uuid)",
    )
    .bind(MAIN)
    .bind(&front_user_id)
    .execute(&desk.db.pool)
    .await
    .expect("insert shift");

    let ledger = xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new();
    for (code, amount) in [("THB", 100_000_i64), ("USD", 1_000_i64)] {
        use xc_cash::domain::posting::CashPostings;
        let mut tx = desk.db.pool.begin().await.expect("begin opening");
        ledger
            .post_in(
                &mut tx,
                &[xc_cash::domain::posting::PostingRequest {
                    from: xc_cash::domain::posting::CashPlace::Bank,
                    to: xc_cash::domain::posting::CashPlace::Drawer("shift-1".to_owned()),
                    currency: xc_kernel::money::CurrencyCode::parse(code).expect("currency"),
                    quantity: xc_kernel::money::ForeignAmount::from_decimal(
                        rust_decimal::Decimal::from(amount),
                    ),
                    source: xc_cash::domain::ledger::posting::SourceRef {
                        source_type: xc_cash::domain::ledger::posting::SourceType::Opening,
                        source_id: "shift-1-opening".to_owned(),
                        source_key: code.to_owned(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("fund shift-1 drawer in ledger");
        tx.commit().await.expect("commit opening");
    }
    // CA-10 (#626, ruling A), setup only: the teller gate refuses a day that is not OPEN (`SAVE_CLOSED`).
    shift_holder_common::open_day(&desk.db.pool, MAIN).await;

    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'cust-1', b.organization_id, 'cust-1', 'Somchai Jaidee', 'LIVE', now() FROM branches b WHERE b.id = $1",
    )
    .bind(MAIN)
    .execute(&desk.db.pool)
    .await
    .expect("insert customer");
    snapshot_common::seed_passport_identity(&desk.db.pool, "cust-1", "AA1234567", "THA").await;

    // C2: the teller who saves uploads the passport (a save attaches its saver's own files)
    desk.files
        .upload(
            &BranchId::new(MAIN),
            &front.id,
            "image/jpeg",
            JPEG,
            Utc::now(),
        )
        .await
        .expect("upload passport")
}

fn claim_call(body: Value) -> Call {
    Call::post(ORIGIN, "/api/device-agent/claim", body.to_string())
        .without("origin")
        .without("x-xchange-request")
        .header("authorization", &format!("Bearer {AGENT_TOKEN}"))
        .header("x-real-ip", "10.1.1.1")
}

/// Construction site 1 (`api/src/presentation/http/devices.rs:43`):
/// A printer model configured in `devices.printerModels` must be accepted by
/// `POST /api/devices` with action `setDrawer` and visible in `snapshot`.
#[actix_web::test]
async fn configured_printer_model_is_accepted_by_devices_route() {
    let desk = Desk::open().await;
    let (app, _, cookies) = setup_app_and_state(&desk).await;
    let owner = &cookies["OWNER"];

    // Set drawer with custom configured model
    let answer = app
        .send(
            Call::action(
                ORIGIN,
                "/api/devices",
                "setDrawer",
                json!({ "printerModel": CUSTOM_MODEL, "hasDrawer": true }),
            )
            .cookie(owner),
        )
        .await;

    assert_eq!(
        answer.status, 200,
        "POST /api/devices setDrawer must accept configured model: {}",
        answer.text
    );
    assert_eq!(answer.json(), json!({ "saved": true }));

    // Snapshot verifies the model is saved and recognized
    let snapshot_ans = app
        .send(Call::action(ORIGIN, "/api/devices", "snapshot", json!({})).cookie(owner))
        .await;
    assert_eq!(snapshot_ans.status, 200, "{}", snapshot_ans.text);
    let snapshot = snapshot_ans.json();
    assert_eq!(snapshot["printerModel"], CUSTOM_MODEL);
    assert_eq!(snapshot["hasDrawer"], true);

    desk.finish().await;
}

/// Construction site 2 (`api/src/presentation/http/rpc.rs:300`):
/// DeviceService built for RPC SaveAction must use configured `printerModels`.
/// When an exchange is saved as cash, DeviceService queues a DRAWER_REQUEST job
/// carrying the configured pulse kick bytes, claimable by the branch device agent.
#[actix_web::test]
async fn configured_printer_model_used_by_rpc_save_action_and_queues_pulse_job() {
    let desk = Desk::open().await;
    let (app, state, cookies) = setup_app_and_state(&desk).await;

    // Ensure SaveAction was registered
    let actions = registered_actions(&state);
    assert!(
        actions
            .iter()
            .any(|a| a.handles(RpcRoute::Rpc, "saveTransaction")),
        "SaveAction must be registered"
    );

    // Seed drawer settings for MAIN to use CUSTOM_MODEL
    sqlx::query(
        "INSERT INTO device_settings (branch_id, key, value) VALUES ($1, 'printerModel', $2), ($1, 'hasDrawer', '1') \
         ON CONFLICT (branch_id, key) DO UPDATE SET value = excluded.value",
    )
    .bind(MAIN)
    .bind(CUSTOM_MODEL)
    .execute(&desk.db.pool)
    .await
    .expect("set device settings");

    // Setup active agent and prerequisites for cash save
    setup_active_agent(&desk).await;
    let passport_key = setup_cash_save_prerequisites(&desk).await;

    // Execute cash saveTransaction through /api/rpc
    let save_payload = json!({
        "branchId": MAIN,
        "submissionId": "00000000-0000-4000-8000-000000000001",
        "customerId": "cust-1",
        "name": "Somchai Jaidee",
        "passportNo": "AA1234567",
        "nationality": "THA",
        "birthDate": "",
        "passportExpiry": "",
        "mode": "LIVE",
        "type": "BUY",
        "currency": "USD",
        "amount": "100",
        "rate": "32.1",
        "fee": "0",
        "bankFee": "0",
        "method": "Cash",
        "ref": "",
        "account": "Cash drawer 1",
        "staff": "desk",
        "booking": "WALK-IN",
        "notes": "",
        "valueDate": "2026-10-06",
        "paidAt": "2026-10-06T05:30:00.000Z",
        "paymentConfirmed": true,
        "identityConfirmed": true,
        "countConfirmed": true,
        "passport": { "uploadKey": passport_key },
    });

    let answer = app
        // C2: FRONT saves
        .send(Call::action(ORIGIN, RPC, "saveTransaction", save_payload).cookie(&cookies["FRONT"]))
        .await;

    assert_eq!(
        answer.status, 200,
        "saveTransaction must succeed: {}",
        answer.text
    );
    assert_eq!(answer.json()["saved"], true);

    // Verify a DRAWER_REQUEST job was queued in device_jobs
    let job_row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, kind, status FROM device_jobs WHERE branch_id = $1 AND kind = 'DRAWER_REQUEST'",
    )
    .bind(MAIN)
    .fetch_optional(&desk.db.pool)
    .await
    .expect("query device_jobs");

    let (job_id, kind, status) = job_row.expect(
        "DRAWER_REQUEST job must be queued when configured printer model has a drawer pulse",
    );
    assert_eq!(kind, "DRAWER_REQUEST");
    assert_eq!(status, "QUEUED");

    // Agent claims the job and verifies configured kick bytes
    let claim_body = json!({
        "waitMs": 0,
        "version": "1.0.1",
        "build": "def5678",
        "os": "macos-14.6-arm64",
        "host": "counter-1.local",
        "drawer": { "transport": "CUPS", "target": "POS80_" }
    });

    let claim_ans = app.send(claim_call(claim_body)).await;
    assert_eq!(
        claim_ans.status, 200,
        "claim must return the queued job: {}",
        claim_ans.text
    );

    let claim_json = claim_ans.json();
    assert_eq!(claim_json["jobId"], job_id);

    assert_eq!(
        claim_json["bytes"], CUSTOM_KICK_BASE64,
        "claim bytes must match the configured model's pulse kick bytes"
    );

    desk.finish().await;
}
