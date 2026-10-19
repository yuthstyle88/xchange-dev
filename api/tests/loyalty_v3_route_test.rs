//! LY-3 route-level wiring test (lane controller note on #1385, issuecomment-6121607729):
//! Asserts that `EXCHANGE_EFFECTS -> effects(state) -> TransactionCompleteCommand`
//! completes an exchange through the v3 route (`POST /api/transactions/{id}/complete`)
//! with the loyalty effect registered and writes its `loyalty.exchange_completed` outbox row.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, test as atest, web};
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use uuid::Uuid;
use xc_auth::domain::access::Role;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::transaction::ExchangeTransaction;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ME: &str = "teller";

struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn callers<'a>(
        &'a self,
        req: &'a actix_web::HttpRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        let staff = match spec.as_str() {
            "teller" => Some(test_callers::staff(
                TELLER,
                &[Role::Teller],
                BranchId::new("MAIN"),
            )),
            _ => None,
        };
        Box::pin(async move { test_callers::callers(None, staff) })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn complete(c: &Counter, id: &str, who: &str, key: Option<&str>) -> Answer {
    let dir = c.scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        c.desk.db.pool.clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::post()
        .uri(&format!("/api/transactions/{id}/complete"))
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(json!({}).to_string());
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    if let Some(key) = key {
        req = req.insert_header(("idempotency-key", key));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn id_of(t: &ExchangeTransaction) -> String {
    t.id.0.to_string()
}

#[derive(Debug, sqlx::FromRow)]
struct OutboxRow {
    organization_id: Uuid,
    event_type: String,
    destination: String,
    aggregate_type: String,
    aggregate_id: String,
    idempotency_key: String,
    payload_redacted: Value,
}

#[actix_web::test]
async fn completing_exchange_via_v3_route_triggers_loyalty_effect_and_enqueues_outbox_event() {
    let c = Counter::start().await;
    let pool = &c.desk.db.pool;

    // Seed ACTIVE verified PHONE for CUSTOMER
    let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(pool)
        .await
        .expect("organization id of MAIN");

    sqlx::query(
        "INSERT INTO customer_identities ( \
             id, customer_id, organization_id, type, encrypted_value, key_id, \
             lookup_hash, verified_at, status, created_at, updated_at \
         ) VALUES ($1, $2, $3, 'PHONE', $4, 'k1', $5, now(), 'ACTIVE', now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(CUSTOMER)
    .bind(org)
    .bind(unique("cipher").into_bytes())
    .bind(unique("lookup").into_bytes())
    .execute(pool)
    .await
    .expect("insert active verified phone for CUSTOMER");

    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    let tx_id = id_of(&t);

    let answer = complete(&c, &tx_id, ME, Some(&unique("idem-complete"))).await;
    assert_eq!(answer.status, StatusCode::OK, "body: {}", answer.text);
    assert_eq!(answer.body["status"], "COMPLETED");

    let rows: Vec<OutboxRow> = sqlx::query_as(
        "SELECT organization_id, event_type, destination, aggregate_type, aggregate_id, \
                idempotency_key, payload_redacted \
         FROM outbox_events WHERE destination = 'loyalty' AND aggregate_id = $1",
    )
    .bind(&tx_id)
    .fetch_all(pool)
    .await
    .expect("read loyalty outbox rows");

    assert_eq!(rows.len(), 1, "exactly one loyalty row: {rows:?}");
    let row = &rows[0];
    assert_eq!(row.organization_id, org);
    assert_eq!(row.event_type, "loyalty.exchange_completed");
    assert_eq!(row.destination, "loyalty");
    assert_eq!(row.aggregate_type, "transaction");
    assert_eq!(row.aggregate_id, tx_id);
    assert_eq!(row.idempotency_key, format!("exchange:{tx_id}"));
    assert_eq!(
        row.payload_redacted,
        json!({ "transactionId": tx_id }),
        "references-only payload: {row:?}"
    );

    c.finish().await;
}
