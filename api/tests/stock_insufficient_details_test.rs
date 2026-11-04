//! Ticket #6092 (F4 BE): `stockInsufficient` refusals carry the shortfall as `error.details` (currency, place, shortBy).
//!
//! Behaviour table from ticket part 9:
//! - B1: create (`POST /api/transactions`) for a cash SELL beyond the drawer
//! - B2: `complete` refused by the ledger port, two short places
//! - B3: a refusal whose tail does not parse (`StockShortfall::default()`)
//! - B4: `ledger_adjustment_post`'s own `stockInsufficient` (unchanged / not this ticket)

mod common;
mod quote_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::body::MessageBody;
use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, ResponseError, test as atest, web};
use chrono::{TimeZone, Utc};
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::domain::shortfall::{ShortPlace, Shortfall, StockShortfall};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::dto::v3::stock_shortfall::{StockShortfallDetails, StockShortfallLine};
use xchange::presentation::http::v3::negative_stock::{
    shortfall_message, stock_insufficient, stock_refusal_answer,
};
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
}

struct Room {
    db: TestDb,
    teller: Who,
    manager: Who,
    customer_id: String,
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

        let scratch = std::env::temp_dir().join(unique("xc-shortfall"));
        std::fs::create_dir_all(scratch.join("uploads")).expect("create scratch dir");

        let room = Self {
            db,
            teller,
            manager,
            customer_id,
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

/// Helper to extract `error_response` body from a `V3Error`.
fn v3_error_body(err: &xchange::presentation::http::v3::error::V3Error) -> (StatusCode, Value) {
    let resp = err.error_response();
    let status = resp.status();
    let bytes = resp
        .into_body()
        .try_into_bytes()
        .unwrap_or_else(|_| panic!("a body in one piece"));
    let json: Value = serde_json::from_slice(&bytes).expect("json body");
    (status, json)
}

/// B1: create (`POST /api/transactions`) for a cash SELL beyond the drawer returns 409
/// `stockInsufficient` with `details.lines = [{currency, place: "DRAWER", shortBy}]`.
#[actix_web::test]
async fn b1_create_for_a_cash_sell_beyond_the_drawer_carries_shortfall_details() {
    let room = Room::start().await;
    // Quote a cash SELL of 1 USD (drawer holds 0 USD).
    let q = room
        .call(
            &room.teller,
            Method::POST,
            "/api/quotes",
            Some(json!({
                "branch": "MAIN",
                "customerId": room.customer_id,
                "paymentMethod": "Cash",
                "lines": [{ "shopSide": "SELL", "currency": "USD", "quantity": "1" }]
            })),
        )
        .await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    let quote_id = q.body["id"].as_str().expect("quote id").to_owned();

    let c = room
        .call(
            &room.teller,
            Method::POST,
            "/api/transactions",
            Some(json!({
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
            })),
        )
        .await;

    assert_eq!(
        (c.status, c.code()),
        (StatusCode::CONFLICT, "stockInsufficient"),
        "{}",
        c.text
    );
    assert_eq!(
        c.body["error"]["message"],
        json!("ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: USD ลิ้นชัก ขาด 1"),
        "{}",
        c.text
    );

    let details: StockShortfallDetails = serde_json::from_value(c.body["error"]["details"].clone())
        .unwrap_or_else(|e| {
            panic!(
                "details deserializes into StockShortfallDetails ({e}): {}",
                c.text
            )
        });

    assert_eq!(
        details.lines,
        vec![StockShortfallLine {
            currency: "USD".to_owned(),
            place: "DRAWER".to_owned(),
            short_by: "1".to_owned(),
        }]
    );

    room.finish().await;
}

/// B2: `complete` refused by the ledger port, two short places: `details.lines` in
/// `StockShortfall` order, two entries.
#[actix_web::test]
async fn b2_complete_refused_by_the_ledger_port_carries_shortfall_details_in_order() {
    let gen_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/shared/api/generated");
    xchange::presentation::dto::v3::stock_shortfall::export_types(&gen_dir)
        .expect("export stock_shortfall.ts");

    let shortfall = StockShortfall {
        lines: vec![
            Shortfall {
                currency: CurrencyCode::parse("USD").unwrap(),
                place: ShortPlace::Drawer,
                short_by: Decimal::from(7),
            },
            Shortfall {
                currency: CurrencyCode::parse("EUR").unwrap(),
                place: ShortPlace::Safe,
                short_by: Decimal::from(15),
            },
        ],
    };
    let refusal_error = shortfall.refusal();
    let refusal = stock_insufficient(&refusal_error).expect("stock_insufficient");
    let err = stock_refusal_answer(refusal);

    let (status, body) = v3_error_body(&err);
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "stockInsufficient");
    assert_eq!(body["error"]["message"], shortfall_message(&shortfall));

    let details: StockShortfallDetails = serde_json::from_value(body["error"]["details"].clone())
        .unwrap_or_else(|e| {
            panic!("details deserializes into StockShortfallDetails ({e}): {body}")
        });

    assert_eq!(
        details.lines,
        vec![
            StockShortfallLine {
                currency: "USD".to_owned(),
                place: "DRAWER".to_owned(),
                short_by: "7".to_owned(),
            },
            StockShortfallLine {
                currency: "EUR".to_owned(),
                place: "SAFE".to_owned(),
                short_by: "15".to_owned(),
            },
        ],
        "lines preserved in StockShortfall order"
    );
}

/// B3: a refusal whose tail does not parse (`StockShortfall::default()`): no `details` key.
#[actix_web::test]
async fn b3_a_refusal_whose_tail_does_not_parse_has_no_details_key() {
    // 1. Without the marker.
    let refusal_no_marker =
        "stockInsufficient: the posting takes a drawer, a safe or a position below zero";
    let err_no_marker = stock_refusal_answer(refusal_no_marker);
    let (status, body_no_marker) = v3_error_body(&err_no_marker);
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body_no_marker["error"]["code"], "stockInsufficient");
    assert!(
        body_no_marker["error"].get("details").is_none(),
        "no details key when no marker: {body_no_marker}"
    );

    // 2. With corrupt JSON tail.
    let refusal_corrupt_tail = format!(
        "stockInsufficient: the posting takes a drawer, a safe or a position below zero{}not-json",
        StockShortfall::MARKER
    );
    let err_corrupt = stock_refusal_answer(&refusal_corrupt_tail);
    let (status2, body_corrupt) = v3_error_body(&err_corrupt);
    assert_eq!(status2, StatusCode::CONFLICT);
    assert_eq!(body_corrupt["error"]["code"], "stockInsufficient");
    assert!(
        body_corrupt["error"].get("details").is_none(),
        "no details key when tail cannot parse: {body_corrupt}"
    );
}
