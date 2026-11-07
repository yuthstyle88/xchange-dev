//! Ticket CA-1f (`docs/design/xc-counter-app.md` §4 rule A, §5.3, §5.4, §8.5): rule A and drawer targeting through the
//! real save paths: v3 create → `confirm` → `complete`, the v2 compat `saveTransaction`, and `POST /api/drawer-passport`.
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! | Session | Counter / branch setting | Exchange | Required result |
//! |---|---|---|---|
//! | bound | `cash_drawer = true` | cash | saved; its `DRAWER_REQUEST` job has `target_agent_id` = that counter |
//! | bound | `cash_drawer` false or NULL | cash | refused `COUNTER_NO_DRAWER` + the §4 message; nothing saved, no drawer job |
//! | bound | `cash_drawer` false or NULL | non-cash | saves normally |
//! | bound | the counter is revoked | cash | refused `COUNTER_NO_DRAWER` |
//! | unbound | `cashFromBrowser` not set, or `{"value":true}` | cash | saved; drawer job with `target_agent_id` NULL (today's behaviour) |
//! | unbound | `cashFromBrowser` `{"value":false}` | cash | refused `COUNTER_NO_DRAWER` |
//! | unbound | no active counter with `cash_drawer` on (a NULL one does not count) | cash | saved, no drawer job (`queue_drawer_in` answers `NoAgent`) |
//! | either | `POST /api/drawer-passport` | | the same rule for its session; `NoAgent` answers `SIMULATED` |
//!
//! How `NoAgent` is seen: through the routes, not by calling `queue_drawer_in`, because §8 fixes no signature for it
//! (it takes no target today, `device_service.rs:286`, and CA-1f must give it one).
//!
//! Where v3 refuses is the implementer's choice among create, confirm and complete, so a run stops at the first answer
//! that is not 200 and the test holds that answer to the code and the message. "Nothing saved" for v3 is: nothing numbered,
//! confirmed or completed (a DRAFT that create already wrote is not a save), and no job of any kind.
//!
//! Through the server's whole route table with real sessions (`rpc_common`). The binding is written straight into
//! `user_sessions.counter_agent_id`, so nothing here depends on CA-1c.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

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
const CODE: &str = "COUNTER_NO_DRAWER";
const MESSAGE: &str = "เคาน์เตอร์นี้ไม่มีลิ้นชักเงิน — ส่งไปชำระที่เคาน์เตอร์ที่มีลิ้นชัก";
const MODEL: &str = "CUSTOM_POS_80";

const COUNTER: &str = "c0000000-0000-4000-8000-000000000001";
const OTHER_COUNTER: &str = "c0000000-0000-4000-8000-000000000002";

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
    /// MAIN with the saver's open shift, a drawer model with a pulse, USD published, a LIVE customer and the saver's
    /// passport scan. No counter yet; the session is unbound.
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
        // The route waits this long for an agent's report; nobody claims here.
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
        // OR-4b: ExchangeTransact is TELLER's, so FRONT saves.
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

    /// An active counter of MAIN with `cash_drawer` as given.
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

    async fn revoke(&self, id: &str) {
        sqlx::query(
            "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = $1::uuid",
        )
        .bind(id)
        .execute(&self.desk.db.pool)
        .await
        .expect("revoke the counter");
    }

    /// FRONT's session is bound to `id`, as `deskBindCounter` would leave it.
    async fn bind(&self, id: &str) {
        let done = sqlx::query(
            "UPDATE user_sessions SET counter_agent_id = $1::uuid \
             WHERE user_id = $2::uuid",
        )
        .bind(id)
        .bind(&self.front)
        .execute(&self.desk.db.pool)
        .await
        .expect("bind the session");
        assert_eq!(done.rows_affected(), 1, "FRONT has one session");
    }

    /// The branch setting `cashFromBrowser`, as `branchCashFromBrowserSave` stores it.
    async fn cash_from_browser(&self, value: bool) {
        sqlx::query(
            "INSERT INTO branch_settings (branch_id, name, version, body, updated_at, updated_by) \
             VALUES ('MAIN', 'cashFromBrowser', 1, $1, now(), 'owner')",
        )
        .bind(json!({ "value": value }))
        .execute(&self.desk.db.pool)
        .await
        .expect("set cashFromBrowser");
    }

    /// `target_agent_id` of every `DRAWER_REQUEST` job.
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

    /// Nothing of a sale exists: nothing numbered or past DRAFT, no job of any kind, no device line.
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

    /// A non-cash payment's slip, the saver's own upload; none for cash.
    async fn slip_of(&self, method: &str) -> Value {
        if method == "Cash" {
            return Value::Null;
        }
        let key = self
            .desk
            .files
            .upload(
                &BranchId::new(MAIN),
                &self.front,
                "image/jpeg",
                JPEG,
                Utc::now(),
            )
            .await
            .expect("upload the slip");
        json!({ "uploadKey": key })
    }

    /// Quote, create, confirm, complete, in the saver's session. `Ok` with the completed answer; `Err` with the first
    /// answer that is not 200. The quote comes before any of the save, so its refusal is a fault of this harness.
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
                        "reference": if method == "Cash" { "" } else { "REF-1" },
                        "account": if method == "Cash" { "Cash drawer 1" } else { "Bank 1" },
                        "slip": self.slip_of(method).await,
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

    /// The v2 compat save through `/api/rpc`, as `configured_printer_models_test.rs`.
    async fn sell_v2(&self, method: &str) -> Answer {
        let cash = method == "Cash";
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
                        "ref": if cash { "" } else { "REF-1" },
                        "account": if cash { "Cash drawer 1" } else { "Bank 1" },
                        "staff": "desk",
                        "booking": "WALK-IN",
                        "notes": "",
                        "valueDate": "2026-10-06",
                        "paidAt": "2026-10-06T05:30:00.000Z",
                        "paymentConfirmed": true,
                        "identityConfirmed": true,
                        "countConfirmed": true,
                        "passport": { "uploadKey": self.passport },
                        "slip": self.slip_of(method).await,
                    }),
                )
                .cookie(&self.cookies["FRONT"]),
            )
            .await
    }

    /// `POST /api/drawer-passport` for an order of the desk.
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

const NON_CASH: &str = "Bank Transfer";

/// A v3 refusal of rule A: `error.code` and `error.message`, as `commands/transactions.rs::refusal` answers.
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

/// A v2 refusal of rule A: v2's answer is the sentence (`rpc_common::Answer::assert_refused`). It is held to the §4
/// message, with or without the code in front of it, and is not a save.
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

// ── bound to a counter with a drawer ────────────────────────────────────────

#[actix_web::test]
async fn v3_bound_to_a_drawer_counter_saves_cash_and_the_drawer_job_targets_it() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;
    room.counter(OTHER_COUNTER, Some(true)).await;
    room.bind(COUNTER).await;

    assert_completed_v3(room.sell_v3("Cash").await, "cash at a drawer counter");

    assert_eq!(room.drawer_jobs().await, [Some(COUNTER.to_owned())]);
    room.finish().await;
}

#[actix_web::test]
async fn v2_bound_to_a_drawer_counter_saves_cash_and_the_drawer_job_targets_it() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;
    room.counter(OTHER_COUNTER, Some(true)).await;
    room.bind(COUNTER).await;

    assert_saved_v2(&room.sell_v2("Cash").await, "cash at a drawer counter");

    assert_eq!(room.drawer_jobs().await, [Some(COUNTER.to_owned())]);
    room.finish().await;
}

// ── rule A: bound to a counter with no drawer ───────────────────────────────

#[actix_web::test]
async fn v3_bound_to_a_counter_with_the_drawer_off_or_not_set_refuses_cash() {
    for setting in [Some(false), None] {
        let room = Room::open().await;
        room.counter(COUNTER, setting).await;
        room.counter(OTHER_COUNTER, Some(true)).await;
        room.bind(COUNTER).await;
        let label = format!("cash_drawer {setting:?}");

        assert_rule_a_v3(room.sell_v3("Cash").await, &label);

        room.assert_nothing_saved(&label).await;
        room.finish().await;
    }
}

#[actix_web::test]
async fn v2_bound_to_a_counter_with_the_drawer_off_or_not_set_refuses_cash() {
    for setting in [Some(false), None] {
        let room = Room::open().await;
        room.counter(COUNTER, setting).await;
        room.counter(OTHER_COUNTER, Some(true)).await;
        room.bind(COUNTER).await;
        let label = format!("cash_drawer {setting:?}");

        assert_rule_a_v2(&room.sell_v2("Cash").await, &label);

        room.assert_nothing_saved(&label).await;
        room.finish().await;
    }
}

/// §4: "the counter can take customers, key, scan and issue queue tickets"; only cash is refused.
#[actix_web::test]
async fn v3_a_non_cash_exchange_saves_normally_at_a_counter_with_no_drawer() {
    let room = Room::open().await;
    room.counter(COUNTER, None).await;
    room.bind(COUNTER).await;

    assert_completed_v3(room.sell_v3(NON_CASH).await, "non-cash");

    assert_eq!(room.drawer_jobs().await, Vec::<Option<String>>::new());
    room.finish().await;
}

#[actix_web::test]
async fn v2_a_non_cash_exchange_saves_normally_at_a_counter_with_no_drawer() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(false)).await;
    room.bind(COUNTER).await;

    assert_saved_v2(&room.sell_v2(NON_CASH).await, "non-cash");

    assert_eq!(room.drawer_jobs().await, Vec::<Option<String>>::new());
    room.finish().await;
}

/// `counter_policy`: a bound counter that is revoked counts as no drawer, even with `cash_drawer` on.
#[actix_web::test]
async fn a_bound_counter_that_was_revoked_counts_as_no_drawer() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;
    room.bind(COUNTER).await;
    // Revoked straight in the row, so the binding is left behind; the rule must hold whether or not revoke clears it.
    room.revoke(COUNTER).await;

    assert_rule_a_v3(room.sell_v3("Cash").await, "v3");
    assert_rule_a_v2(&room.sell_v2("Cash").await, "v2");

    room.assert_nothing_saved("revoked").await;
    room.finish().await;
}

// ── unbound: the branch setting decides ─────────────────────────────────────

/// §5.4: not set = on, today's behaviour.
#[actix_web::test]
async fn unbound_with_the_setting_not_set_saves_cash_with_no_target() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;

    assert_completed_v3(room.sell_v3("Cash").await, "v3");
    assert_saved_v2(&room.sell_v2("Cash").await, "v2");

    assert_eq!(room.drawer_jobs().await, [None, None]);
    room.finish().await;
}

#[actix_web::test]
async fn unbound_with_the_setting_on_saves_cash_with_no_target() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;
    room.cash_from_browser(true).await;

    assert_completed_v3(room.sell_v3("Cash").await, "v3");
    assert_saved_v2(&room.sell_v2("Cash").await, "v2");

    assert_eq!(room.drawer_jobs().await, [None, None]);
    room.finish().await;
}

#[actix_web::test]
async fn unbound_with_the_setting_off_refuses_cash() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;
    room.cash_from_browser(false).await;

    assert_rule_a_v3(room.sell_v3("Cash").await, "v3");
    assert_rule_a_v2(&room.sell_v2("Cash").await, "v2");

    room.assert_nothing_saved("off").await;
    room.finish().await;
}

/// Off refuses cash, not everything.
#[actix_web::test]
async fn unbound_with_the_setting_off_still_saves_non_cash() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;
    room.cash_from_browser(false).await;

    assert_completed_v3(room.sell_v3(NON_CASH).await, "v3");
    assert_saved_v2(&room.sell_v2(NON_CASH).await, "v2");
    room.finish().await;
}

// ── NoAgent: a NULL-target job needs a counter with its drawer on ───────────

/// §5.2: NULL = off, so a counter with `cash_drawer` NULL does not count, and the agent paired on xc-v3 today claims no
/// drawer job until the Owner turns it on. The sale itself is not refused: the branch allows cash from a browser.
#[actix_web::test]
async fn unbound_with_no_active_drawer_counter_saves_cash_and_queues_no_drawer_job() {
    let room = Room::open().await;
    room.counter(COUNTER, None).await;
    room.counter(OTHER_COUNTER, Some(false)).await;

    assert_completed_v3(room.sell_v3("Cash").await, "v3");
    assert_saved_v2(&room.sell_v2("Cash").await, "v2");

    assert_eq!(room.drawer_jobs().await, Vec::<Option<String>>::new());
    room.finish().await;
}

/// A revoked counter with its drawer on is no active counter.
#[actix_web::test]
async fn unbound_with_only_a_revoked_drawer_counter_queues_no_drawer_job() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;
    room.revoke(COUNTER).await;

    assert_saved_v2(&room.sell_v2("Cash").await, "v2");

    assert_eq!(room.drawer_jobs().await, Vec::<Option<String>>::new());
    room.finish().await;
}

// ── the passport drawer ─────────────────────────────────────────────────────

#[actix_web::test]
async fn the_passport_drawer_bound_to_a_drawer_counter_targets_it() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;
    room.counter(OTHER_COUNTER, Some(true)).await;
    room.bind(COUNTER).await;

    let answer = room.passport_drawer().await;

    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(room.drawer_jobs().await, [Some(COUNTER.to_owned())]);
    room.finish().await;
}

#[actix_web::test]
async fn the_passport_drawer_bound_to_a_counter_with_no_drawer_is_refused() {
    for setting in [Some(false), None] {
        let room = Room::open().await;
        room.counter(COUNTER, setting).await;
        room.counter(OTHER_COUNTER, Some(true)).await;
        room.bind(COUNTER).await;
        let label = format!("cash_drawer {setting:?}");

        let answer = room.passport_drawer().await;

        assert!(
            answer.text.contains(MESSAGE),
            "{label}: the §4 message: {} {:.300}",
            answer.status,
            answer.text
        );
        assert_eq!(
            room.drawer_jobs().await,
            Vec::<Option<String>>::new(),
            "{label}"
        );
        room.finish().await;
    }
}

#[actix_web::test]
async fn the_passport_drawer_unbound_with_the_setting_off_is_refused() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;
    room.cash_from_browser(false).await;

    let answer = room.passport_drawer().await;

    assert!(
        answer.text.contains(MESSAGE),
        "the §4 message: {} {:.300}",
        answer.status,
        answer.text
    );
    assert_eq!(room.drawer_jobs().await, Vec::<Option<String>>::new());
    room.finish().await;
}

/// Today's behaviour for a browser, with no target.
#[actix_web::test]
async fn the_passport_drawer_unbound_with_the_setting_not_set_queues_with_no_target() {
    let room = Room::open().await;
    room.counter(COUNTER, Some(true)).await;

    let answer = room.passport_drawer().await;

    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(room.drawer_jobs().await, [None]);
    room.finish().await;
}

/// `NoAgent` is `SIMULATED` on this route (`passport_drawer_device.rs:108`): a counter with `cash_drawer` NULL does not
/// count as an agent for a NULL-target job.
#[actix_web::test]
async fn the_passport_drawer_with_no_active_drawer_counter_answers_simulated() {
    let room = Room::open().await;
    room.counter(COUNTER, None).await;

    let answer = room.passport_drawer().await;

    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.json()["state"], "SIMULATED", "{}", answer.text);
    assert_eq!(room.drawer_jobs().await, Vec::<Option<String>>::new());
    room.finish().await;
}
