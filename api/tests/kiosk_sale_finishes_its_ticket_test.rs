//! F25 (#564 issuecomment-6657891388, customer simulation on xc-v3 at 7caf1e7, 2026-11-05): a kiosk sale leaves
//! its queue ticket SERVING (W001, W002), so the next call answers 409 `alreadyServing` until the teller presses
//! "เสร็จสิ้น". The desk opened from `ทำรายการต่อ` names the kiosk request (`kioskRequestId`) and may not name the
//! ticket (`create.rs` `check_queue_ticket_in`: `queueTicketId` with `kioskRequestId` is refused, WQ-3 §5). The
//! request's ticket is `queue_tickets.kiosk_request_id` (UNIQUE per branch, `20261101100000_queue.sql`), but
//! `complete.rs` looks a ticket up only for `booking_id` (F3), so a kiosk request's ticket is never finished.
//!
//! Written before the implementation by the ticket's author (claude/33691c06, sub-controller of the xchange lane
//! controller claude/022a7453). Do not edit, delete or weaken these tests; add your own in another file.
//!
//! [required] by these tests: when `complete` completes a sale whose `kioskRequestId` names a request with a SERVING
//! queue ticket, that ticket is finished in the same transaction (SERVING → COMPLETED, `completed_at` set, one
//! `FINISH` event), exactly as the booking's ticket is (F3). A request whose ticket was already finished, or that has
//! no ticket, still completes; the teller's walk-in ticket for someone else is untouched.
//!
//! The harness is `booked_sale_finishes_its_ticket_test.rs`'s, copied. The kiosk request and its ticket are written
//! with SQL in the shape the kiosk, the queue machine and the desk's accept leave them. Needs `TEST_DATABASE_URL`;
//! nothing skips.

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

// ── the kiosk request and its ticket ───────────────────────────────────────────────────────────────────────────

/// An ID_CHECKED kiosk request (the desk's accept and passport check done) held by the room's teller, matching `quote` (shop BUY USD 1, cash), with a queue ticket
/// in `ticket_status` held by the teller (`None`: no ticket). Answers `(request id, ticket id)`.
async fn requested(room: &Room, ticket_status: Option<&str>) -> (String, Option<String>) {
    let pool = &room.db.pool;
    let request = format!("K-{}", Uuid::new_v4().simple().to_string()[..8].to_uppercase());
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, rate, \
             foreign_amount, thb, input_amount, payment_method) \
         VALUES ('MAIN', $1, 'ID_CHECKED', now(), $2, 'BUY', 'USD', 35.0000, 1.00, 35.00, 1.00, 'Cash')",
    )
    .bind(&request)
    .bind(&room.teller.id)
    .execute(pool)
    .await
    .expect("seed an ID_CHECKED kiosk request");
    let Some(status) = ticket_status else {
        return (request, None);
    };
    let ticket = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, \
             status, kiosk_request_id, counter_teller_id, issued_at, called_at, served_at, completed_at, idempotency_key) \
         SELECT $1::uuid, organization_id, 'MAIN', current_date, 'WALK_IN', 7, 'W007', $2, $3, $4::uuid, \
             now(), now(), CASE WHEN $2 IN ('SERVING','COMPLETED') THEN now() END, \
             CASE WHEN $2 = 'COMPLETED' THEN now() END, $5 \
         FROM branches WHERE id = 'MAIN'",
    )
    .bind(&ticket)
    .bind(status)
    .bind(&request)
    .bind(&room.teller.id)
    .bind(format!("f25-{ticket}"))
    .execute(pool)
    .await
    .expect("seed the kiosk request's ticket");
    (request, Some(ticket))
}

async fn request_status(room: &Room, request: &str) -> String {
    sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE branch_id = 'MAIN' AND id = $1")
        .bind(request)
        .fetch_one(&room.db.pool)
        .await
        .expect("read the kiosk request")
}

async fn kiosk_sale(room: &Room, request: &str) -> String {
    let quote_id = quote(room).await;
    let created = create(room, &quote_id, json!({ "kioskRequestId": request })).await;
    assert_eq!(
        created.status,
        StatusCode::OK,
        "create with kioskRequestId: {}",
        created.text
    );
    let tx_id = created.body["id"]
        .as_str()
        .expect("transaction id")
        .to_owned();
    let done = confirm_and_complete(room, &tx_id).await;
    assert_eq!(done.status, StatusCode::OK, "complete: {}", done.text);
    tx_id
}

#[actix_web::test]
async fn f25_completing_a_kiosk_sale_finishes_the_requests_serving_ticket() {
    let room = Room::start().await;
    let (request, ticket) = requested(&room, Some("SERVING")).await;
    let ticket = ticket.expect("a ticket");

    kiosk_sale(&room, &request).await;

    assert_eq!(
        ticket_state(&room, &ticket).await,
        ("COMPLETED".to_owned(), true, 1),
        "the kiosk request's ticket is finished with one FINISH event, as a booking's is (F3)"
    );
    assert_eq!(
        request_status(&room, &request).await,
        "COMPLETED",
        "the kiosk request is COMPLETED with the sale, unchanged"
    );
    room.finish().await;
}

#[actix_web::test]
async fn f25_a_request_whose_ticket_was_already_finished_still_completes() {
    let room = Room::start().await;
    let (request, ticket) = requested(&room, Some("COMPLETED")).await;
    let ticket = ticket.expect("a ticket");

    kiosk_sale(&room, &request).await;

    assert_eq!(
        ticket_state(&room, &ticket).await,
        ("COMPLETED".to_owned(), true, 0),
        "a finished ticket is not finished again"
    );
    assert_eq!(request_status(&room, &request).await, "COMPLETED");
    room.finish().await;
}

#[actix_web::test]
async fn f25_a_request_with_no_ticket_still_completes() {
    let room = Room::start().await;
    let (request, _) = requested(&room, None).await;

    kiosk_sale(&room, &request).await;

    assert_eq!(request_status(&room, &request).await, "COMPLETED");
    room.finish().await;
}

#[actix_web::test]
async fn f25_the_tellers_walk_in_ticket_is_not_touched_by_a_kiosk_sale() {
    let room = Room::start().await;
    let walk_in = serve(&room).await;
    let (request, _) = requested(&room, None).await;

    kiosk_sale(&room, &request).await;

    assert_eq!(
        ticket_state(&room, &walk_in).await,
        ("SERVING".to_owned(), false, 0),
        "only the kiosk request's own ticket is finished"
    );
    room.finish().await;
}
