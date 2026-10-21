//! Test for ticket #400 rework:
//! Two branches with different device settings: each branch's view shows only its own settings,
//! and a config whose first printer model is not NEOCAL makes the view and /api/devices answer the same model.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xchange::bootstrap::AppState;
use xchange::infrastructure::devices::printer_models::{DrawerPulse, PrinterModel};

const ORIGIN: &str = "http://desk.test";
const DEVICES: &str = "/api/devices";
const MAIN: &str = "MAIN";
const B9: &str = "B9";
const B9_NAME: &str = "Nine";

const CUSTOM_MODEL: &str = "CUSTOM_POS_80";
const OTHER_MODEL: &str = "OTHER_POS_58";

static NEXT_CALL: AtomicU64 = AtomicU64::new(1);
fn next() -> u64 {
    NEXT_CALL.fetch_add(1, Ordering::Relaxed)
}

fn custom_models() -> Vec<PrinterModel> {
    vec![
        PrinterModel {
            id: CUSTOM_MODEL.to_owned(),
            name: "Custom POS-80".to_owned(),
            paper_mm: 80,
            drawer_pulse: Some(DrawerPulse {
                pin: 0,
                on: 30,
                off: 200,
            }),
        },
        PrinterModel {
            id: OTHER_MODEL.to_owned(),
            name: "Other POS-58".to_owned(),
            paper_mm: 58,
            drawer_pulse: None,
        },
    ]
}

struct SettingsRoom {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

impl SettingsRoom {
    async fn open() -> Self {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, B9_NAME).await;

        let mut settings = (*desk.state().settings).clone();
        settings.devices.printer_models = custom_models();
        let state = AppState::new(Arc::new(settings), desk.db.pool.clone());

        let clock: Arc<dyn xchange::presentation::http::rpc::RpcClock> = desk.clock.clone();
        let bank: Arc<dyn xc_rate::domain::rate::entities::rate_source::RateSource> =
            desk.bank.clone();
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

        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);

        SettingsRoom {
            desk,
            app,
            cookies,
            t0,
        }
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
            .without("x-xchange-request")
            .without("content-type")
            .without("origin");
        call.method = Method::GET;
        let call = call.cookie(&self.cookies[who]);
        self.app.send(call).await
    }

    async fn view_data(&self, who: &str, path: &str) -> Value {
        let answer = self.get(who, path).await;
        assert_eq!(
            answer.status, 200,
            "GET {path} as {who:?}: {:.400}",
            answer.text
        );
        let body = answer.json();
        assert_eq!(body["changed"], json!(true));
        body["data"].clone()
    }

    async fn devices(&self, who: &str, action: &str, payload: Value) -> Value {
        self.tick();
        let call = Call::action(ORIGIN, DEVICES, action, payload).cookie(&self.cookies[who]);
        let answer = self.app.send(call).await;
        assert_eq!(
            answer.status, 200,
            "devices {action} as {who:?}: {:.400}",
            answer.text
        );
        answer.json()
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

#[actix_web::test]
async fn two_branches_different_settings_and_non_default_model_registry() {
    let room = SettingsRoom::open().await;

    // 1. Initial state: neither branch has custom settings stored.
    // The configured first model is CUSTOM_MODEL ("CUSTOM_POS_80"), not NEOCAL.
    // Both the view and /api/devices must answer CUSTOM_MODEL for both branches.
    let main_view_init = room
        .view_data("OWNER", "/api/v3/views/devices?branch=MAIN")
        .await;
    let main_dev_init = room
        .devices("OWNER", "snapshot", json!({ "branchId": MAIN }))
        .await;
    assert_eq!(main_view_init["printerModel"], json!(CUSTOM_MODEL));
    assert_eq!(main_dev_init["printerModel"], json!(CUSTOM_MODEL));
    assert_eq!(
        main_view_init["printerModel"], main_dev_init["printerModel"],
        "unconfigured MAIN view and /api/devices must answer the same model"
    );

    let b9_view_init = room
        .view_data("OWNER", "/api/v3/views/devices?branch=B9")
        .await;
    let b9_dev_init = room
        .devices("OWNER", "snapshot", json!({ "branchId": B9 }))
        .await;
    assert_eq!(b9_view_init["printerModel"], json!(CUSTOM_MODEL));
    assert_eq!(b9_dev_init["printerModel"], json!(CUSTOM_MODEL));
    assert_eq!(
        b9_view_init["printerModel"], b9_dev_init["printerModel"],
        "unconfigured B9 view and /api/devices must answer the same model"
    );

    // 2. Set different device settings for MAIN and B9:
    // MAIN: SIMULATED printer, online: true, CUSTOM_MODEL with drawer: true
    room.devices(
        "OWNER",
        "setPrinter",
        json!({ "branchId": MAIN, "kind": "SIMULATED" }),
    )
    .await;
    room.devices(
        "OWNER",
        "printerOnline",
        json!({ "branchId": MAIN, "online": true }),
    )
    .await;
    room.devices(
        "OWNER",
        "setDrawer",
        json!({ "branchId": MAIN, "printerModel": CUSTOM_MODEL, "hasDrawer": true }),
    )
    .await;

    // B9: NETWORK printer, online: false, OTHER_MODEL with drawer: false
    room.devices(
        "OWNER",
        "setPrinter",
        json!({ "branchId": B9, "kind": "NETWORK", "host": "192.168.1.100", "port": 9100 }),
    )
    .await;
    room.devices(
        "OWNER",
        "printerOnline",
        json!({ "branchId": B9, "online": false }),
    )
    .await;
    room.devices(
        "OWNER",
        "setDrawer",
        json!({ "branchId": B9, "printerModel": OTHER_MODEL, "hasDrawer": false }),
    )
    .await;

    // 3. Verify each branch's view shows only its own settings,
    // and matches /api/devices snapshot on the same branch.
    let main_view = room
        .view_data("OWNER", "/api/v3/views/devices?branch=MAIN")
        .await;
    let main_dev = room
        .devices("OWNER", "snapshot", json!({ "branchId": MAIN }))
        .await;

    assert_eq!(main_view["printerModel"], json!(CUSTOM_MODEL));
    assert_eq!(main_view["printerKind"], json!("SIMULATED"));
    assert_eq!(main_view["printerOnline"], json!(true));
    assert_eq!(main_view["hasDrawer"], json!(true));
    assert_eq!(main_dev["printerModel"], json!(CUSTOM_MODEL));
    assert_eq!(main_dev["printerOnline"], json!(true));
    assert_eq!(main_dev["hasDrawer"], json!(true));
    assert_eq!(
        main_view["printerModel"], main_dev["printerModel"],
        "MAIN view and /api/devices must answer the same model"
    );

    let b9_view = room
        .view_data("OWNER", "/api/v3/views/devices?branch=B9")
        .await;
    let b9_dev = room
        .devices("OWNER", "snapshot", json!({ "branchId": B9 }))
        .await;

    assert_eq!(b9_view["printerModel"], json!(OTHER_MODEL));
    assert_eq!(b9_view["printerKind"], json!("NETWORK"));
    assert_eq!(b9_view["printerOnline"], json!(false));
    assert_eq!(b9_view["hasDrawer"], json!(false));
    assert_eq!(b9_dev["printerModel"], json!(OTHER_MODEL));
    assert_eq!(b9_dev["printerOnline"], json!(false));
    assert_eq!(b9_dev["hasDrawer"], json!(false));
    assert_eq!(
        b9_view["printerModel"], b9_dev["printerModel"],
        "B9 view and /api/devices must answer the same model"
    );

    // 4. Verify cross-branch isolation:
    // MAIN settings did not bleed into B9 and vice versa
    assert_ne!(main_view["printerModel"], b9_view["printerModel"]);
    assert_ne!(main_view["printerKind"], b9_view["printerKind"]);
    assert_ne!(main_view["printerOnline"], b9_view["printerOnline"]);
    assert_ne!(main_view["hasDrawer"], b9_view["hasDrawer"]);

    room.finish().await;
}
