//! Ticket CA-1f (#6388): rule A and drawer targeting when the same teller has two live sessions.
//!
//! 1. The request's session is bound to a no-drawer counter and the other live session (with a
//!    later expiry) is bound to a drawer counter: both a v2 and a v3 cash sale are refused
//!    `COUNTER_NO_DRAWER`, with nothing saved and no drawer job.
//! 2. Reverse case: an unbound request while a second live session is bound follows
//!    `cashFromBrowser`, with a `NULL` target when allowed and `COUNTER_NO_DRAWER` when off.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
#[macro_use]
mod rpc_common;
mod shift_holder_common;
mod snapshot_common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, MAIN, RPC, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::AppState;
use xchange::infrastructure::devices::printer_models::{DrawerPulse, PrinterModel};

const ORIGIN: &str = "http://desk.test";
const CODE: &str = "COUNTER_NO_DRAWER";
const MESSAGE: &str = "เคาน์เตอร์นี้ไม่มีลิ้นชักเงิน — ส่งไปชำระที่เคาน์เตอร์ที่มีลิ้นชัก";
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
    second_session_hash: String,
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

        // Seed a second live session for FRONT with a later expiry than the request's session.
        let second_token = {
            let mut conn = desk.db.pool.acquire().await.expect("acquire connection");
            xc_testdb::sessions::seed_session(
                &mut conn,
                &front,
                Some(MAIN),
                Utc::now() + Duration::hours(2),
            )
            .await
            .expect("seed second session")
        };
        let second_session_hash = xc_auth::application::session::sha256_hex(&second_token);

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
            second_session_hash,
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

    /// Binds FRONT's request session (`self.cookies["FRONT"]`) to `id`.
    async fn bind_request_session(&self, id: &str) {
        let done = sqlx::query(
            "UPDATE user_sessions SET counter_agent_id = $1::uuid \
             WHERE user_id = $2::uuid AND hash != $3",
        )
        .bind(id)
        .bind(&self.front)
        .bind(&self.second_session_hash)
        .execute(&self.desk.db.pool)
        .await
        .expect("bind request session");
        assert_eq!(done.rows_affected(), 1);
    }

    /// Binds FRONT's second live session to `id`.
    async fn bind_second_session(&self, id: &str) {
        let done =
            sqlx::query("UPDATE user_sessions SET counter_agent_id = $1::uuid WHERE hash = $2")
                .bind(id)
                .bind(&self.second_session_hash)
                .execute(&self.desk.db.pool)
                .await
                .expect("bind second session");
        assert_eq!(done.rows_affected(), 1);
    }

    async fn cash_from_browser(&self, value: bool) {
        sqlx::query(
            "INSERT INTO branch_settings (branch_id, name, version, body, updated_at, updated_by) \
             VALUES ('MAIN', 'cashFromBrowser', 1, $1, now(), 'owner') \
             ON CONFLICT (branch_id, name) DO UPDATE SET version = branch_settings.version + 1, body = excluded.body",
        )
        .bind(json!({ "value": value }))
        .execute(&self.desk.db.pool)
        .await
        .expect("set cashFromBrowser");
    }

    async fn drawer_jobs(&self) -> Vec<Option<String>> {
        sqlx::query_scalar(
            "SELECT target_agent_id::text FROM device_jobs WHERE kind = 'DRAWER_REQUEST' ORDER BY created_at, id",
        )
        .fetch_all(&self.desk.db.pool)
        .await
        .expect("read the drawer jobs")
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

    async fn sell_v3(&self, method: &str) -> Result<Answer, Answer> {
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
        if created.status != 200 {
            return Err(created);
        }
        let id = created.json()["id"].as_str().expect("an id").to_owned();
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

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn assert_rule_a_v3(got: Result<Answer, Answer>, label: &str) {
    let answer = match got {
        Err(answer) => answer,
        Ok(answer) => panic!(
            "{label}: the sale must be refused, it completed: {:.300}",
            answer.text
        ),
    };
    let body = answer.json();
    assert_eq!(body["error"]["code"], CODE, "{label}: {}", answer.text);
    assert_eq!(
        body["error"]["message"], MESSAGE,
        "{label}: {}",
        answer.text
    );
}

fn assert_rule_a_v2(answer: &Answer, label: &str) {
    assert_ne!(answer.status, 200, "{label}: {:.300}", answer.text);
    assert!(
        answer.text.contains(MESSAGE),
        "{label}: the §4 message: {:.300}",
        answer.text
    );
}

fn assert_saved_v2(answer: &Answer, label: &str) {
    assert_eq!(answer.status, 200, "{label}: {:.300}", answer.text);
    assert_eq!(
        answer.json()["saved"],
        true,
        "{label}: {:.300}",
        answer.text
    );
}

fn assert_completed_v3(got: Result<Answer, Answer>, label: &str) {
    match got {
        Ok(answer) => assert_eq!(
            answer.json()["status"],
            "COMPLETED",
            "{label}: {}",
            answer.text
        ),
        Err(answer) => panic!("{label}: {} {:.400}", answer.status, answer.text),
    }
}

#[actix_web::test]
async fn request_session_bound_to_no_drawer_counter_with_second_session_bound_to_drawer_counter_refuses_cash()
 {
    let room = Room::open().await;
    room.counter(NO_DRAWER_COUNTER, Some(false)).await;
    room.counter(DRAWER_COUNTER, Some(true)).await;
    room.bind_request_session(NO_DRAWER_COUNTER).await;
    room.bind_second_session(DRAWER_COUNTER).await;

    assert_rule_a_v3(room.sell_v3("Cash").await, "v3");
    assert_rule_a_v2(&room.sell_v2("Cash").await, "v2");

    room.assert_nothing_saved("request bound to no-drawer counter")
        .await;
    room.finish().await;
}

#[actix_web::test]
async fn unbound_request_with_bound_second_session_follows_cash_from_browser() {
    let room = Room::open().await;
    room.counter(NO_DRAWER_COUNTER, Some(false)).await;
    room.counter(DRAWER_COUNTER, Some(true)).await;
    room.bind_second_session(DRAWER_COUNTER).await;

    // cashFromBrowser not set (defaults to allowed): saves with a NULL target, not DRAWER_COUNTER.
    assert_completed_v3(room.sell_v3("Cash").await, "v3 unbound with setting on");
    assert_saved_v2(&room.sell_v2("Cash").await, "v2 unbound with setting on");
    assert_eq!(room.drawer_jobs().await, [None, None]);

    // cashFromBrowser false: both v3 and v2 cash sales are refused COUNTER_NO_DRAWER.
    room.cash_from_browser(false).await;
    assert_rule_a_v3(room.sell_v3("Cash").await, "v3 unbound with setting off");
    assert_rule_a_v2(&room.sell_v2("Cash").await, "v2 unbound with setting off");
    assert_eq!(room.drawer_jobs().await, [None, None]);

    room.finish().await;
}
