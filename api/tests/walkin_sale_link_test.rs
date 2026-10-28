//! Up-front tests for WQ-3, the desk sale's link to its walk-in queue ticket (design `docs/design/walkin-queue.md`
//! §4 class diagram, §5 refusals, §6 data migration, W8; §8 row `walkin_sale_link_test.rs`).
//!
//! Written before the implementation by the ticket author claude/e2926a49 (sub-controller XC-WALKIN-QUEUE under
//! the xchange BE lane controller claude/553782). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What WQ-3 makes true:
//! - `POST /api/transactions` takes `queueTicketId` and stores it in `transactions.queue_ticket_id` (§6).
//! - It refuses `validationFailed`, field `queueTicketId`, when the body also names `bookingId` or
//!   `kioskRequestId`; when no such ticket is in the branch; when the ticket is not SERVING or not held by the
//!   caller; when the ticket has a booking or a kiosk request (§5).
//! - A second non-CANCELLED transaction naming the same ticket is `staleVersion` (the unique index of §6).
//! - `complete` finishes the ticket (SERVING → COMPLETED with a `FINISH` event) in the same transaction (W8), and
//!   still completes the sale when the ticket was already finished from the board.
//! - A sale without `queueTicketId` is unchanged: the column is NULL and no queue ticket moves. (A booking sale
//!   keeps its own path; its existing tests stay green unedited.)
//!
//! The ticket is taken the way the desk takes it: `POST /api/queue/tickets/serve` (WQ-2). Setup the routes do
//! not offer (a ticket another teller holds, one already finished, one with a kiosk request) is written with SQL.
//!
//! Harness copied from `cash_rounding_transaction_test.rs` (real routes, a shift opened by cash counts), plus the
//! queue seeding of `walkin_serve_command_test.rs`. Needs `TEST_DATABASE_URL` naming an existing database whose
//! name ends in `_test`; nothing here skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use pos_infra::transactions::Transactor;
use quote_common::*;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

struct Who {
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG_1,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn field(&self) -> Option<&str> {
        self.body["error"]["field"].as_str()
    }
}

struct Room {
    db: TestDb,
    teller: Who,
    manager: Who,
    customer_id: String,
    shift_id: String,
    scratch: std::path::PathBuf,
}

impl Room {
    async fn start() -> Self {
        PII.call_once(|| {
            let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
            let _ = xc_customer::install_pii_keys(keys);
        });
        let db = TestDb::start().await;
        let now_utc = Utc::now();
        let open_hhmm = (now_utc - chrono::Duration::hours(2))
            .format("%H:%M")
            .to_string();
        let close_hhmm = (now_utc + chrono::Duration::hours(2))
            .format("%H:%M")
            .to_string();
        let day_win = json!([{ "open": open_hhmm, "close": close_hhmm }]);
        let hours = json!({
            "mon": day_win, "tue": day_win, "wed": day_win, "thu": day_win,
            "fri": day_win, "sat": day_win, "sun": day_win
        });
        db.exec(&format!(
            "UPDATE branches SET timezone = 'UTC', opening_hours = '{hours}'::jsonb WHERE id = 'MAIN'"
        ))
        .await;
        db.publish("MAIN", &[USD]).await;
        db.exec(
            "INSERT INTO ops_state (branch_id, phase) VALUES ('MAIN', 'OPEN') \
             ON CONFLICT (branch_id) DO UPDATE SET phase = 'OPEN'",
        )
        .await;

        let mut tx = db.pool.begin().await.expect("begin");
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Safe(BranchId::new("MAIN")),
                    currency: CurrencyCode::THB,
                    quantity: ForeignAmount::from_decimal("1000.00".parse().expect("amount")),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: unique("fund-safe-thb"),
                        source_key: "fund".to_owned(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("fund safe");
        tx.commit().await.expect("commit");

        let teller = Self::seed_person(&db, Role::Teller, Some("MAIN")).await;
        let manager = Self::seed_person(&db, Role::BranchManager, Some("MAIN")).await;

        for (key, sha) in [("PASSPORT-MAIN", "sha-passport"), ("SLIP-MAIN", "sha-slip")] {
            db.exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('{key}', 'MAIN', '{}', 'image/jpeg', '{sha}', now())",
                teller.id
            ))
            .await;
        }

        let customers = CustomerService::new(BranchTx::new(Transactor::new(db.pool.clone())));
        let customer_id = customers
            .reserve(
                &BranchId::new("MAIN"),
                ReserveCustomer {
                    name: "Somchai Jaidee".to_owned(),
                    passport_no: format!("P{}", &Uuid::new_v4().simple().to_string()[..19])
                        .to_uppercase(),
                    nationality: "THA".to_owned(),
                    birth_date: String::new(),
                    passport_expiry: String::new(),
                    mode: "LIVE".to_owned(),
                },
                whole_second_now(),
            )
            .await
            .expect("reserve LIVE customer")
            .customer_id;

        let scratch = std::env::temp_dir().join(unique("xc-wq3"));
        std::fs::create_dir_all(scratch.join("uploads")).expect("create scratch dir");

        let mut room = Self {
            db,
            teller,
            manager,
            customer_id,
            shift_id: String::new(),
            scratch,
        };

        // Open MAIN's shift with THB 20 x 2 = 40.00 through the cash-counts routes.
        let thb20 = room.denomination("THB", "20").await;
        let open = room
            .call(
                &room.teller,
                Method::POST,
                "/api/cash-counts",
                Some(json!({
                    "branch": "MAIN",
                    "kind": "OPEN",
                    "lines": [{ "currency": "THB", "denominationId": thb20, "qty": "2" }]
                })),
            )
            .await;
        assert_eq!(open.status, StatusCode::OK, "{}", open.text);
        let open_id = open.body["id"].as_str().expect("open count id");

        let verified = room
            .call(
                &room.manager,
                Method::POST,
                &format!("/api/cash-counts/{open_id}/verify"),
                Some(json!({ "branch": "MAIN" })),
            )
            .await;
        assert_eq!(verified.status, StatusCode::OK, "{}", verified.text);

        room.shift_id = sqlx::query_scalar(
            "SELECT id::text FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
        )
        .bind(&room.teller.id)
        .fetch_one(&room.db.pool)
        .await
        .expect("open shift id");

        // The desk queue (as walkin_serve_command_test.rs seeds it): the teller's ACTIVE counter, an open queue.
        room.db
            .exec(&format!(
                "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
                 VALUES ('{ORG_1}'::uuid, 'MAIN', '{}'::uuid, 'ACTIVE') \
                 ON CONFLICT (branch_id, teller_id) DO NOTHING",
                room.teller.id
            ))
            .await;
        room.db
            .exec(&format!(
                "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, \
                     queue_version) VALUES ('MAIN', '{ORG_1}'::uuid, true, 1, 1) \
                 ON CONFLICT (branch_id) DO UPDATE SET queue_open = true, active_counters = 1, queue_version = 1"
            ))
            .await;

        room
    }

    async fn seed_person(db: &TestDb, role: Role, branch: Option<&str>) -> Who {
        let id = xc_testdb::people::seed_person(
            &mut db.pool.acquire().await.expect("acquire"),
            ORG_1,
            role.as_str(),
            &[xc_testdb::people::RoleRow {
                role_code: role.as_str(),
                branch_id: branch,
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            }],
        )
        .await
        .expect("seed person");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
        }
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.db.pool)
        .await
        .unwrap_or_else(|e| panic!("denomination {currency} {face}: {e}"))
    }

    async fn call(&self, who: &Who, method: Method, uri: &str, body: Option<Value>) -> Answer {
        let uploads = self.scratch.join("uploads");
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_FILES__DIR",
                uploads.to_str().expect("UTF-8 path"),
            )])),
            self.db.pool.clone(),
        );
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let mut req = atest::TestRequest::default()
            .method(method)
            .uri(uri)
            .insert_header((header::HOST, HOST))
            .insert_header((AS, who.spec.clone()));
        if let Some(b) = &body {
            req = req
                .insert_header(("x-xchange-request", "1"))
                .insert_header((header::ORIGIN, SAME_ORIGIN))
                .insert_header((header::CONTENT_TYPE, "application/json"))
                .insert_header(("idempotency-key", unique("key")))
                .set_payload(b.to_string());
        }
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer { status, text, body }
    }

    async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
        self.db.finish().await;
    }
}

// ── the sale and its ticket ─────────────────────────────────────────────────────────────────────────────────────

const SERVE: &str = "/api/queue/tickets/serve";

/// The teller serves the customer at the counter (WQ-2): the SERVING ticket's id.
async fn serve(room: &Room) -> String {
    let served = room
        .call(
            &room.teller,
            Method::POST,
            SERVE,
            Some(json!({ "branch": "MAIN", "customerId": null })),
        )
        .await;
    assert_eq!(served.status, StatusCode::OK, "serve: {}", served.text);
    assert_eq!(served.body["status"], json!("SERVING"), "{}", served.text);
    served.body["id"].as_str().expect("ticket id").to_owned()
}

/// A cash BUY USD 1 quote for the room's customer: its id.
async fn quote(room: &Room) -> String {
    let quote = room
        .call(
            &room.teller,
            Method::POST,
            "/api/quotes",
            Some(json!({
                "branch": "MAIN",
                "customerId": room.customer_id,
                "paymentMethod": "Cash",
                "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
            })),
        )
        .await;
    assert_eq!(quote.status, StatusCode::OK, "quote: {}", quote.text);
    quote.body["id"].as_str().expect("quote id").to_owned()
}

/// `POST /api/transactions` for `quote_id`, with `extra` merged into the body (e.g. `queueTicketId`).
async fn create(room: &Room, quote_id: &str, extra: Value) -> Answer {
    let mut body = json!({
        "branch": "MAIN",
        "quoteId": quote_id,
        "customerId": room.customer_id,
        "passport": { "uploadKey": "PASSPORT-MAIN" },
        "payment": {
            "method": "Cash",
            "reference": "",
            "account": "Cash drawer 1",
            "slip": null,
            "bankFeeThb": "0.00",
            "paidAt": "2026-10-06T09:00:00Z"
        },
        "valueDate": "2026-10-06",
        "notes": "",
        "countConfirmed": true
    });
    for (k, v) in extra.as_object().expect("extra is an object") {
        body[k] = v.clone();
    }
    room.call(&room.teller, Method::POST, "/api/transactions", Some(body))
        .await
}

/// Confirm then complete `tx_id`: the complete answer.
async fn confirm_and_complete(room: &Room, tx_id: &str) -> Answer {
    let confirmed = room
        .call(
            &room.teller,
            Method::POST,
            &format!("/api/transactions/{tx_id}/confirm"),
            Some(json!({ "branch": "MAIN" })),
        )
        .await;
    assert_eq!(
        confirmed.status,
        StatusCode::OK,
        "confirm: {}",
        confirmed.text
    );
    room.call(
        &room.teller,
        Method::POST,
        &format!("/api/transactions/{tx_id}/complete"),
        Some(json!({ "branch": "MAIN" })),
    )
    .await
}

/// The stored `transactions.queue_ticket_id` of `tx_id` (the column is WQ-3's migration).
async fn stored_ticket(room: &Room, tx_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT queue_ticket_id::text FROM transactions WHERE id = $1::uuid")
        .bind(tx_id)
        .fetch_one(&room.db.pool)
        .await
        .unwrap_or_else(|e| panic!("read transactions.queue_ticket_id of {tx_id}: {e}"))
}

/// `(status, completed_at is set, FINISH events)` of a queue ticket.
async fn ticket_state(room: &Room, ticket: &str) -> (String, bool, i64) {
    let (status, completed): (String, bool) = sqlx::query_as(
        "SELECT status, completed_at IS NOT NULL FROM queue_tickets WHERE id = $1::uuid",
    )
    .bind(ticket)
    .fetch_one(&room.db.pool)
    .await
    .expect("read the ticket");
    let finishes = room
        .db
        .count(&format!(
            "SELECT count(*) FROM queue_events WHERE ticket_id = '{ticket}'::uuid AND kind = 'FINISH'"
        ))
        .await;
    (status, completed, finishes)
}

fn refused_on_queue_ticket(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(a.code(), "validationFailed", "{what}: {}", a.text);
    assert_eq!(a.field(), Some("queueTicketId"), "{what}: {}", a.text);
}

async fn no_transaction(room: &Room, what: &str) {
    assert_eq!(
        room.db.count("SELECT count(*) FROM transactions").await,
        0,
        "{what}: a refusal writes no transaction"
    );
}

// ── stored ──────────────────────────────────────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn wq3_a_create_naming_the_served_ticket_stores_it() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    let q = quote(&room).await;

    let created = create(&room, &q, json!({ "queueTicketId": ticket })).await;

    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    let tx_id = created.body["id"].as_str().expect("tx id");
    assert_eq!(
        stored_ticket(&room, tx_id).await.as_deref(),
        Some(ticket.as_str())
    );
    room.finish().await;
}

#[actix_web::test]
async fn wq3_a_sale_without_a_queue_ticket_is_unchanged() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    let q = quote(&room).await;

    let created = create(&room, &q, json!({})).await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    let tx_id = created.body["id"].as_str().expect("tx id").to_owned();
    assert_eq!(stored_ticket(&room, &tx_id).await, None);

    let completed = confirm_and_complete(&room, &tx_id).await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text);
    assert_eq!(completed.body["status"], json!("COMPLETED"));
    assert_eq!(
        ticket_state(&room, &ticket).await,
        ("SERVING".to_owned(), false, 0),
        "a sale that names no ticket moves none"
    );
    room.finish().await;
}

// ── refused: validationFailed, field queueTicketId (§5) ─────────────────────────────────────────────────────────

#[actix_web::test]
async fn wq3_a_queue_ticket_with_a_booking_id_is_refused() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    let q = quote(&room).await;

    let a = create(
        &room,
        &q,
        json!({ "queueTicketId": ticket, "bookingId": "BK-WQ3" }),
    )
    .await;

    refused_on_queue_ticket(&a, "with bookingId");
    no_transaction(&room, "with bookingId").await;
    room.finish().await;
}

#[actix_web::test]
async fn wq3_a_queue_ticket_with_a_kiosk_request_id_is_refused() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    let q = quote(&room).await;

    let a = create(
        &room,
        &q,
        json!({ "queueTicketId": ticket, "kioskRequestId": "KR-WQ3" }),
    )
    .await;

    refused_on_queue_ticket(&a, "with kioskRequestId");
    no_transaction(&room, "with kioskRequestId").await;
    room.finish().await;
}

#[actix_web::test]
async fn wq3_no_such_ticket_in_the_branch_is_refused() {
    let room = Room::start().await;
    let q = quote(&room).await;

    let a = create(
        &room,
        &q,
        json!({ "queueTicketId": Uuid::new_v4().to_string() }),
    )
    .await;
    refused_on_queue_ticket(&a, "an unknown ticket");

    let b = create(&room, &q, json!({ "queueTicketId": "not-a-uuid" })).await;
    refused_on_queue_ticket(&b, "not a uuid");
    no_transaction(&room, "no such ticket").await;
    room.finish().await;
}

#[actix_web::test]
async fn wq3_a_ticket_that_is_not_serving_is_refused() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    room.db
        .exec(&format!(
            "UPDATE queue_tickets SET status = 'COMPLETED', completed_at = now() WHERE id = '{ticket}'::uuid"
        ))
        .await;
    let q = quote(&room).await;

    let a = create(&room, &q, json!({ "queueTicketId": ticket })).await;

    refused_on_queue_ticket(&a, "a COMPLETED ticket");
    no_transaction(&room, "a COMPLETED ticket").await;
    room.finish().await;
}

#[actix_web::test]
async fn wq3_a_ticket_another_teller_holds_is_refused() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    room.db
        .exec(&format!(
            "UPDATE queue_tickets SET counter_teller_id = '{}'::uuid WHERE id = '{ticket}'::uuid",
            room.manager.id
        ))
        .await;
    let q = quote(&room).await;

    let a = create(&room, &q, json!({ "queueTicketId": ticket })).await;

    refused_on_queue_ticket(&a, "another teller's ticket");
    no_transaction(&room, "another teller's ticket").await;
    room.finish().await;
}

#[actix_web::test]
async fn wq3_a_ticket_with_a_kiosk_request_is_refused() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    room.db
        .exec(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, side, currency, rate, \
                 foreign_amount, thb, input_amount, payment_method) \
             VALUES ('MAIN', 'KR-WQ3', 'WAITING', now(), 'BUY', 'USD', 35.0000, 100.00, 3500.00, 100.00, 'Cash')",
        )
        .await;
    room.db
        .exec(&format!(
            "UPDATE queue_tickets SET kiosk_request_id = 'KR-WQ3' WHERE id = '{ticket}'::uuid"
        ))
        .await;
    let q = quote(&room).await;

    let a = create(&room, &q, json!({ "queueTicketId": ticket })).await;

    refused_on_queue_ticket(&a, "a kiosk request's ticket");
    no_transaction(&room, "a kiosk request's ticket").await;
    room.finish().await;
}

// ── one sale per ticket (§6 unique index) ───────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn wq3_a_second_sale_naming_the_same_ticket_is_stale_version() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    let first = create(
        &room,
        &quote(&room).await,
        json!({ "queueTicketId": ticket }),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);

    let second = create(
        &room,
        &quote(&room).await,
        json!({ "queueTicketId": ticket }),
    )
    .await;

    assert_eq!(second.status, StatusCode::CONFLICT, "{}", second.text);
    assert_eq!(second.code(), "staleVersion", "{}", second.text);
    assert_eq!(
        room.db.count("SELECT count(*) FROM transactions").await,
        1,
        "the second sale is not written"
    );
    room.finish().await;
}

// ── finished with the sale (W8) ─────────────────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn wq3_complete_finishes_the_sales_ticket() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    let created = create(
        &room,
        &quote(&room).await,
        json!({ "queueTicketId": ticket }),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    let tx_id = created.body["id"].as_str().expect("tx id").to_owned();
    assert_eq!(
        ticket_state(&room, &ticket).await,
        ("SERVING".to_owned(), false, 0),
        "create does not finish the ticket"
    );

    let completed = confirm_and_complete(&room, &tx_id).await;

    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text);
    assert_eq!(completed.body["status"], json!("COMPLETED"));
    assert_eq!(
        ticket_state(&room, &ticket).await,
        ("COMPLETED".to_owned(), true, 1),
        "SERVING → COMPLETED with one FINISH event"
    );
    room.finish().await;
}

#[actix_web::test]
async fn wq3_complete_still_succeeds_when_the_ticket_was_finished_from_the_board() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    let created = create(
        &room,
        &quote(&room).await,
        json!({ "queueTicketId": ticket }),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    let tx_id = created.body["id"].as_str().expect("tx id").to_owned();
    // the teller finished it from the board first
    room.db
        .exec(&format!(
            "UPDATE queue_tickets SET status = 'COMPLETED', completed_at = now() WHERE id = '{ticket}'::uuid"
        ))
        .await;

    let completed = confirm_and_complete(&room, &tx_id).await;

    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text);
    assert_eq!(completed.body["status"], json!("COMPLETED"));
    assert_eq!(
        ticket_state(&room, &ticket).await,
        ("COMPLETED".to_owned(), true, 0),
        "the ticket is left as it was: no second finish"
    );
    room.finish().await;
}
