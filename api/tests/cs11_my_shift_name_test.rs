mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use quote_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

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
    manager: Who,
    teller: Who,
    shift_id: String,
}

impl Room {
    async fn start() -> Self {
        let db = TestDb::start().await;
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
            .expect("fund the safe");
        tx.commit().await.expect("commit");
        let manager = Self::seed_person(&db, Role::BranchManager).await;
        let teller = Self::seed_person(&db, Role::Teller).await;
        let mut room = Self {
            db,
            manager,
            teller,
            shift_id: String::new(),
        };
        room.shift_id = room.open_shift(&room.teller).await;
        room
    }

    async fn seed_person(db: &TestDb, role: Role) -> Who {
        let id = xc_testdb::people::seed_person(
            &mut db.pool.acquire().await.expect("acquire"),
            ORG_1,
            role.as_str(),
            &[xc_testdb::people::RoleRow {
                role_code: role.as_str(),
                branch_id: Some("MAIN"),
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            }],
        )
        .await
        .expect("seed person");
        Who {
            spec: format!("{id}|{}|MAIN", role.as_str()),
            id,
        }
    }

    async fn thb20(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'THB' AND face_value = 20::numeric",
        )
        .fetch_one(&self.db.pool)
        .await
        .expect("the THB 20 denomination")
    }

    async fn open_shift(&self, teller: &Who) -> String {
        let thb20 = self.thb20().await;
        let open = self
            .call(
                teller,
                Method::POST,
                "/api/cash-counts",
                Some(json!({
                    "branch": "MAIN",
                    "kind": "OPEN",
                    "lines": [{ "currency": "THB", "denominationId": thb20, "qty": "1" }]
                })),
            )
            .await;
        assert_eq!(open.status, StatusCode::OK, "{}", open.text);
        let open_id = open.body["id"].as_str().expect("open count id").to_owned();
        let verified = self
            .call(
                &self.manager,
                Method::POST,
                &format!("/api/cash-counts/{open_id}/verify"),
                Some(json!({ "branch": "MAIN" })),
            )
            .await;
        assert_eq!(verified.status, StatusCode::OK, "{}", verified.text);
        sqlx::query_scalar(
            "SELECT id::text FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
        )
        .bind(&teller.id)
        .fetch_one(&self.db.pool)
        .await
        .expect("the teller's open shift")
    }

    async fn call(&self, who: &Who, method: Method, uri: &str, body: Option<Value>) -> Answer {
        let state = AppState::new(Arc::new(common::settings(&[])), self.db.pool.clone());
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

    async fn my_shift(&self, who: &Who) -> Value {
        let a = self
            .call(who, Method::GET, "/api/v3/views/my_shift?branch=MAIN", None)
            .await;
        assert_eq!(a.status, StatusCode::OK, "my_shift: {}", a.text);
        a.body["data"].clone()
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

/// R1a: my_shift for a shift opened with template B
#[actix_web::test]
async fn cs11_r1a_my_shift_templated() {
    let room = Room::start().await;
    sqlx::query(
        "UPDATE shifts \
         SET template_id = (SELECT id FROM shift_templates WHERE branch_id = 'MAIN' AND name = 'B'), \
             template_name = 'B', \
             business_date = '2026-11-04'::date \
         WHERE id = $1",
    )
    .bind(&room.shift_id)
    .execute(&room.db.pool)
    .await
    .expect("update shift with template");

    let data = room.my_shift(&room.teller).await;
    assert_eq!(data["shiftName"], json!("B"), "{data}");
    assert_eq!(data["businessDate"], json!("2026-11-04"), "{data}");
    room.finish().await;
}

/// R1c: my_shift for a shift with no template
#[actix_web::test]
async fn cs11_r1c_my_shift_no_template() {
    let room = Room::start().await;
    // Since CS-7 an opened shift takes the covering template, so make this one a pre-feature shift by hand.
    sqlx::query("UPDATE shifts SET template_id = NULL, template_name = NULL WHERE id = $1")
        .bind(&room.shift_id)
        .execute(&room.db.pool)
        .await
        .expect("strip the shift's template");
    let data = room.my_shift(&room.teller).await;
    assert_eq!(data["shiftName"], Value::Null, "{data}");
    assert!(
        data["businessDate"].is_string(),
        "businessDate should be string: {data}"
    );
    room.finish().await;
}
