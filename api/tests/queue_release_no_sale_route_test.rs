//! #5541 (design `docs/design/blueprint/queue-tickets.md` §4.6; owner 2026-10-29 "ยกเลิกได้ ระบุว่าไม่มีการขาย"):
//! `POST /api/queue/tickets/{id}/release` (`ticketRelease`, `QueueOperate`) ends a SERVING ticket whose sale never
//! happened as CANCELLED with reason `noSale`, so the teller can serve the next walk-in. A ticket that a
//! non-CANCELLED transaction names (`transactions.queue_ticket_id`) is refused `ticketHasSale` and stays SERVING.
//! Round 4's W013: the PromptPay save was blocked, the ticket stayed SERVING, and the next walk-in got 409
//! `alreadyServing`.
//!
//! Written before the implementation by the ticket author (claude/2ca5874b). Do not edit, delete or weaken; add tests
//! in a file of your own. The harness is `walkin_sale_link_test.rs`'s, copied so that file stays untouched.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

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

// ── release with no sale ───────────────────────────────────────────────────────────────────────────────────────

async fn release(room: &Room, ticket: &str) -> Answer {
    room.call(
        &room.teller,
        Method::POST,
        &format!("/api/queue/tickets/{ticket}/release"),
        Some(json!({ "branch": "MAIN", "id": ticket })),
    )
    .await
}

async fn ticket_end(room: &Room, ticket: &str) -> (String, Option<String>) {
    sqlx::query_as("SELECT status, ended_reason FROM queue_tickets WHERE id = $1::uuid")
        .bind(ticket)
        .fetch_one(&room.db.pool)
        .await
        .expect("read the ticket")
}

#[actix_web::test]
async fn a_served_walk_in_with_no_sale_is_released_and_the_next_walk_in_is_served() {
    let room = Room::start().await;
    let first = serve(&room).await;

    let released = release(&room, &first).await;

    assert_eq!(released.status, StatusCode::OK, "{}", released.text);
    assert_eq!(
        released.body["status"],
        json!("CANCELLED"),
        "{}",
        released.text
    );
    assert_eq!(
        ticket_end(&room, &first).await,
        ("CANCELLED".to_owned(), Some("noSale".to_owned()))
    );
    assert_eq!(
        room.db
            .count(&format!(
                "SELECT count(*) FROM queue_events WHERE ticket_id = '{first}'::uuid \
                 AND kind = 'CANCEL' AND status = 'CANCELLED' AND reason = 'noSale'"
            ))
            .await,
        1
    );
    let second = serve(&room).await;
    assert_ne!(second, first, "a new ticket for the next customer");
    room.finish().await;
}

#[actix_web::test]
async fn a_ticket_a_sale_names_is_refused_ticket_has_sale_and_stays_serving() {
    let room = Room::start().await;
    let ticket = serve(&room).await;
    let created = create(
        &room,
        &quote(&room).await,
        json!({ "queueTicketId": ticket }),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);

    let refused = release(&room, &ticket).await;

    assert!(refused.status.is_client_error(), "{}", refused.text);
    assert_eq!(refused.code(), "ticketHasSale", "{}", refused.text);
    assert_eq!(
        ticket_end(&room, &ticket).await,
        ("SERVING".to_owned(), None)
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_ticket_whose_only_sale_was_cancelled_is_released() {
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
    let cancelled = room
        .call(
            &room.teller,
            Method::POST,
            &format!("/api/transactions/{tx_id}/cancel"),
            Some(json!({ "branch": "MAIN", "id": tx_id, "reason": "customer left" })),
        )
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{}", cancelled.text);

    let released = release(&room, &ticket).await;

    assert_eq!(released.status, StatusCode::OK, "{}", released.text);
    assert_eq!(
        ticket_end(&room, &ticket).await,
        ("CANCELLED".to_owned(), Some("noSale".to_owned()))
    );
    room.finish().await;
}

#[test]
fn the_policy_row_ticket_release_needs_queue_operate() {
    use xc_auth::domain::access::Permission;
    use xc_auth::domain::policy::entities::policy::{Need, Policy};
    use xc_auth::domain::policy::value_objects::action::Action;
    use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

    assert_eq!(
        Policy::need(EntryPath::Rpc, &Action::new("ticketRelease")),
        Some(Need::Permission(Permission::QueueOperate))
    );
}
