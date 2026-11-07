//! Ticket CA-1h (#6424): rule A fails closed when reading the caller's counter binding fails.
//!
//! - With `user_sessions.counter_agent_id` unreadable (dropped in the test database), a v2 cash
//!   save, a v3 cash save (`confirm` and `complete`), and `POST /api/drawer-passport` answer 500
//!   with no transaction saved and no drawer job queued.
//! - A non-save RPC route (`getConnectionStatus`) and a non-save v3 route (`POST /api/quotes`)
//!   are unaffected and still answer 200.

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
use rpc_common::{Answer, App, Call, Cookies, Desk, MAIN, RPC, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::AppState;
use xchange::infrastructure::devices::printer_models::{DrawerPulse, PrinterModel};

const ORIGIN: &str = "http://desk.test";
const PASSPORT_ROUTE: &str = "/api/drawer-passport";
const SERVER_ERROR_MESSAGE: &str = "เซิร์ฟเวอร์ขัดข้อง";
const MODEL: &str = "CUSTOM_POS_80";

const NO_DRAWER_COUNTER: &str = "c0000000-0000-4000-8000-000000000001";
const DRAWER_COUNTER: &str = "c0000000-0000-4000-8000-000000000002";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    front: String,
    passport: String,
}

impl Room {
    async fn open() -> Room {
        let keys =
            xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .expect("the test PII key parses");
        let _ = xc_customer::install_pii_keys(Arc::new(keys));
        let desk = Desk::open().await;
        let mut settings = (*desk.state().settings).clone();
        settings.devices.printer_models = vec![PrinterModel {
            id: MODEL.to_owned(),
            name: "Custom POS-80".to_owned(),
            paper_mm: 80,
            drawer_pulse: Some(DrawerPulse {
                pin: 0,
                on: 30,
                off: 200,
            }),
        }];
        settings.devices.passport_drawer_branches = vec![MAIN.to_owned()];
        settings.devices.passport_drawer_read_window_ms = 200;
        let state = AppState::new(Arc::new(settings), desk.db.pool.clone());
        let clock: Arc<dyn xchange::presentation::http::rpc::RpcClock> = desk.clock.clone();
        let bank: Arc<dyn xc_rate::domain::rate::entities::rate_source::RateSource> =
            desk.bank.clone();
        let app = App::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state.clone()))
                    .app_data(actix_web::web::Data::from(clock))
                    .app_data(actix_web::web::Data::new(bank)),
            )
            .await,
            |t| t.to_request(),
        );
        let people: Vec<_> = one_of_each_role()
            .into_iter()
            .filter(|p| p.key == "OWNER" || p.key == "FRONT")
            .collect();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let front = people
            .iter()
            .find(|p| p.key == "FRONT")
            .expect("FRONT")
            .id
            .clone();

        sqlx::query(
            "INSERT INTO device_settings (branch_id, key, value) VALUES ($1, 'printerModel', $2), ($1, 'hasDrawer', '1') \
             ON CONFLICT (branch_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(MAIN)
        .bind(MODEL)
        .execute(&desk.db.pool)
        .await
        .expect("set the drawer model");
        rates_re4e_publish::set_counter_rate(&desk.db.pool, MAIN, "USD", "32.1", "33.0").await;

        let shift_teller = shift_holder_common::user_of(&desk.db.pool, &front).await;
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('shift-1', $1, 'OPEN', now(), 'FRONT', 'open-req-1', $2::uuid)",
        )
        .bind(MAIN)
        .bind(&shift_teller)
        .execute(&desk.db.pool)
        .await
        .expect("open the saver's shift");
        for (code, amount) in [("THB", 100_000_i64), ("USD", 1_000_i64)] {
            use xc_cash::domain::posting::CashPostings;
            let mut tx = desk.db.pool.begin().await.expect("begin opening");
            xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
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
                .expect("fund the drawer");
            tx.commit().await.expect("commit opening");
        }
        shift_holder_common::open_day(&desk.db.pool, MAIN).await;

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT 'cust-1', b.organization_id, 'cust-1', 'Somchai Jaidee', 'LIVE', now() FROM branches b WHERE b.id = $1",
        )
        .bind(MAIN)
        .execute(&desk.db.pool)
        .await
        .expect("insert the customer");
        snapshot_common::seed_passport_identity(&desk.db.pool, "cust-1", "AA1234567", "THA").await;
        let passport = desk
            .files
            .upload(&BranchId::new(MAIN), &front, "image/jpeg", JPEG, Utc::now())
            .await
            .expect("upload the passport");
        Room {
            desk,
            app,
            cookies,
            front,
            passport,
        }
    }

    async fn counter(&self, id: &str, cash_drawer: Option<bool>) {
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, paired_at, last_seen_at, cash_drawer) \
             VALUES ($1::uuid, 'MAIN', 'Mac', now(), 'owner', now(), now(), $2)",
        )
        .bind(id)
        .bind(cash_drawer)
        .execute(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert counter {id}: {e}"));
    }

    async fn bind(&self, id: &str) {
        let done = sqlx::query(
            "UPDATE user_sessions SET counter_agent_id = $1::uuid WHERE user_id = $2::uuid",
        )
        .bind(id)
        .bind(&self.front)
        .execute(&self.desk.db.pool)
        .await
        .expect("bind the session");
        assert_eq!(done.rows_affected(), 1);
    }

    async fn break_binding_read(&self) {
        sqlx::query("ALTER TABLE user_sessions DROP COLUMN counter_agent_id")
            .execute(&self.desk.db.pool)
            .await
            .expect("drop counter_agent_id to force a binding-read DB error");
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn assert_nothing_saved(&self, label: &str) {
        assert_eq!(
            self.count(
                "SELECT count(*) FROM transactions WHERE document_ref IS NOT NULL \
                 OR status NOT IN ('DRAFT', 'CANCELLED')"
            )
            .await,
            0,
            "{label}: no exchange is saved"
        );
        assert_eq!(
            self.count("SELECT count(*) FROM device_jobs").await,
            0,
            "{label}: no job of any kind"
        );
    }

    async fn v3(&self, path: &str, body: Value) -> Answer {
        self.app
            .send(
                Call::post(ORIGIN, path, body.to_string())
                    .header("idempotency-key", &uuid::Uuid::new_v4().to_string())
                    .cookie(&self.cookies["FRONT"]),
            )
            .await
    }

    async fn quote_and_create_v3(&self, method: &str) -> String {
        let quote = self
            .v3(
                "/api/quotes",
                json!({
                    "branch": MAIN,
                    "customerId": "cust-1",
                    "paymentMethod": method,
                    "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }],
                }),
            )
            .await;
        assert_eq!(quote.status, 200, "the quote: {}", quote.text);
        let quote = quote.json();
        let created = self
            .v3(
                "/api/transactions",
                json!({
                    "branch": MAIN,
                    "quoteId": quote["id"],
                    "customerId": "cust-1",
                    "passport": { "uploadKey": self.passport },
                    "payment": {
                        "method": method,
                        "reference": "",
                        "account": "Cash drawer 1",
                        "slip": Value::Null,
                        "bankFeeThb": "0.00",
                        "paidAt": Utc::now().to_rfc3339(),
                    },
                    "notes": "",
                    "countConfirmed": true,
                }),
            )
            .await;
        assert_eq!(created.status, 200, "the draft: {}", created.text);
        created.json()["id"].as_str().expect("an id").to_owned()
    }

    async fn sell_v3(&self, method: &str) -> Result<Answer, Answer> {
        let id = self.quote_and_create_v3(method).await;
        let confirmed = self
            .v3(
                &format!("/api/transactions/{id}/confirm"),
                json!({ "branch": MAIN }),
            )
            .await;
        if confirmed.status != 200 {
            return Err(confirmed);
        }
        let completed = self
            .v3(
                &format!("/api/transactions/{id}/complete"),
                json!({ "branch": MAIN }),
            )
            .await;
        if completed.status != 200 {
            return Err(completed);
        }
        Ok(completed)
    }

    async fn sell_v2(&self, method: &str) -> Answer {
        self.app
            .send(
                Call::action(
                    ORIGIN,
                    RPC,
                    "saveTransaction",
                    json!({
                        "branchId": MAIN,
                        "submissionId": uuid::Uuid::new_v4().to_string(),
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
                        "method": method,
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
                        "passport": { "uploadKey": self.passport },
                        "slip": Value::Null,
                    }),
                )
                .cookie(&self.cookies["FRONT"]),
            )
            .await
    }

    async fn rpc(&self, action: &str, payload: Value) -> Answer {
        self.app
            .send(Call::action(ORIGIN, RPC, action, payload).cookie(&self.cookies["FRONT"]))
            .await
    }

    async fn passport_drawer(&self) -> Answer {
        let order = uuid::Uuid::new_v4().to_string();
        let body = json!({ "branchId": MAIN, "orderId": order, "kiosk": false, "mode": "LIVE" });
        self.app
            .send(
                Call::post(ORIGIN, PASSPORT_ROUTE, body.to_string()).cookie(&self.cookies["FRONT"]),
            )
            .await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn assert_v3_server_error(got: Result<Answer, Answer>, label: &str) {
    let answer = match got {
        Err(answer) => answer,
        Ok(answer) => panic!(
            "{label}: the save must fail with 500, it completed: {:.300}",
            answer.text
        ),
    };
    assert_eq!(answer.status, 500, "{label}: {}", answer.text);
    let body = answer.json();
    assert_eq!(
        body["error"]["code"], "serverError",
        "{label}: {}",
        answer.text
    );
    assert_eq!(
        body["error"]["message"], SERVER_ERROR_MESSAGE,
        "{label}: {}",
        answer.text
    );
}

fn assert_v2_server_error(answer: &Answer, label: &str) {
    assert_eq!(answer.status, 500, "{label}: {:.300}", answer.text);
    assert_eq!(
        answer.json(),
        json!({ "error": SERVER_ERROR_MESSAGE }),
        "{label}: {:.300}",
        answer.text
    );
}

#[actix_web::test]
async fn binding_read_db_error_refuses_v2_and_v3_cash_save_with_500_and_leaves_non_save_routes_unaffected()
 {
    let room = Room::open().await;
    room.counter(NO_DRAWER_COUNTER, Some(false)).await;
    room.counter(DRAWER_COUNTER, Some(true)).await;
    room.bind(NO_DRAWER_COUNTER).await;

    room.break_binding_read().await;

    // Non-RPC-save routes (both RPC non-save and v3 non-save) are unaffected.
    let status_rpc = room
        .rpc("getConnectionStatus", json!({ "branchId": MAIN }))
        .await;
    assert_eq!(
        status_rpc.status, 200,
        "non-save RPC route is unaffected: {}",
        status_rpc.text
    );

    // v3 and v2 cash saves and the passport drawer route fail closed with 500.
    assert_v3_server_error(room.sell_v3("Cash").await, "v3 cash save");
    assert_v2_server_error(&room.sell_v2("Cash").await, "v2 cash save");
    assert_v2_server_error(&room.passport_drawer().await, "passport drawer");

    room.assert_nothing_saved("binding read DB error").await;
    room.finish().await;
}

#[actix_web::test]
async fn binding_read_db_error_at_v3_complete_answers_500_with_no_completed_transaction_or_drawer_job()
 {
    let room = Room::open().await;
    room.counter(DRAWER_COUNTER, Some(true)).await;
    room.bind(DRAWER_COUNTER).await;

    let id = room.quote_and_create_v3("Cash").await;
    let confirmed = room
        .v3(
            &format!("/api/transactions/{id}/confirm"),
            json!({ "branch": MAIN }),
        )
        .await;
    assert_eq!(
        confirmed.status, 200,
        "confirm before break: {}",
        confirmed.text
    );

    room.break_binding_read().await;

    let completed = room
        .v3(
            &format!("/api/transactions/{id}/complete"),
            json!({ "branch": MAIN }),
        )
        .await;
    assert_v3_server_error(Err(completed), "v3 complete");

    assert_eq!(
        room.count(
            "SELECT count(*) FROM transactions WHERE document_ref IS NOT NULL OR status = 'COMPLETED'"
        )
        .await,
        0,
        "no transaction completed"
    );
    assert_eq!(
        room.count("SELECT count(*) FROM device_jobs").await,
        0,
        "no drawer job exists"
    );
    room.finish().await;
}

#[actix_web::test]
async fn caller_bound_counter_and_bound_counter_in_propagate_db_error_when_user_sessions_is_unreadable_and_allow_no_cookie()
 {
    use actix_web::http::header;
    use actix_web::test::TestRequest;
    use xchange::presentation::extractors::actor::{
        bound_counter_in, caller_bound_counter, try_caller_session_hash,
    };

    let room = Room::open().await;
    room.counter(NO_DRAWER_COUNTER, Some(false)).await;
    room.counter(DRAWER_COUNTER, Some(true)).await;
    room.bind(NO_DRAWER_COUNTER).await;
    let draft_id = room.quote_and_create_v3("Cash").await;

    let now = Utc::now();
    let state = actix_web::web::Data::new(room.desk.state().clone());

    sqlx::query("ALTER TABLE user_sessions RENAME TO user_sessions_gone")
        .execute(&room.desk.db.pool)
        .await
        .expect("rename user_sessions");

    // No session cookie stays unbound (Ok(None)) without touching the database.
    let no_cookie_req = TestRequest::default()
        .app_data(state.clone())
        .to_http_request();
    assert_eq!(
        try_caller_session_hash(&no_cookie_req, now)
            .await
            .expect("no cookie has no session hash"),
        None
    );
    assert_eq!(
        caller_bound_counter(&no_cookie_req, now)
            .await
            .expect("no cookie stays unbound"),
        None
    );

    // With a session cookie, a failed read of user_sessions returns Err, never Ok(None).
    let cookie_req = TestRequest::default()
        .insert_header((header::COOKIE, room.cookies["FRONT"].as_str()))
        .app_data(state)
        .to_http_request();
    assert!(
        try_caller_session_hash(&cookie_req, now).await.is_err(),
        "try_caller_session_hash must fail closed when user_sessions cannot be read"
    );
    assert!(
        caller_bound_counter(&cookie_req, now).await.is_err(),
        "caller_bound_counter must fail closed when user_sessions cannot be read"
    );

    // v3 caller authentication also answers 500 serverError when the session-hash read fails.
    let confirmed = room
        .v3(
            &format!("/api/transactions/{draft_id}/confirm"),
            json!({ "branch": MAIN }),
        )
        .await;
    assert_v3_server_error(Err(confirmed), "v3 confirm with unreadable user_sessions");

    let mut tx = room.desk.db.pool.begin().await.expect("begin tx");
    assert_eq!(
        bound_counter_in(&mut tx, None)
            .await
            .expect("no session hash stays unbound"),
        None
    );
    assert!(
        bound_counter_in(&mut tx, Some("any-hash")).await.is_err(),
        "bound_counter_in must fail closed when user_sessions cannot be read"
    );
    drop(tx);

    sqlx::query("ALTER TABLE user_sessions_gone RENAME TO user_sessions")
        .execute(&room.desk.db.pool)
        .await
        .expect("restore user_sessions");
    room.finish().await;
}
