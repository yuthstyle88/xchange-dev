//! Shared by the FD-3 (#974) route tests: feed rows seeded straight into FD-1's table, and a GET of
//! the route as Broker sends it. Written by the ticket author; do not edit, delete or weaken.
#![allow(dead_code)]

use actix_web::http::Method;
use serde_json::Value;

use crate::rpc_common::{Answer, App, Call, Desk};

pub const PATH: &str = "/api/v3/system/broker/exchanges";
pub const HOST: &str = "desk.test";

/// A GET of the route with `query` (without `?`), with `token` as `X-System-Token` when given.
pub fn get(query: &str, token: Option<&str>) -> Call {
    let mut headers = std::collections::BTreeMap::new();
    if let Some(t) = token {
        headers.insert("x-system-token".to_string(), t.to_string());
    }
    let path = if query.is_empty() {
        PATH.to_string()
    } else {
        format!("{PATH}?{query}")
    };
    Call {
        method: Method::GET,
        host: HOST.to_string(),
        path,
        headers,
        body: Vec::new(),
    }
}

pub async fn send(app: &App, call: Call) -> Answer {
    app.send(call).await
}

/// One stored exchange of `MAIN` (a DEMO one needs no shift; the feed does not read `mode`),
/// with a customer of its own. Answers the exchange's id.
pub async fn exchange(desk: &Desk, n: u32) -> String {
    let customer = format!("CUST-FD3-{n}");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT $1, b.organization_id, $1, $1, 'LIVE', now() FROM branches b WHERE b.id = 'MAIN'",
    )
    .bind(&customer)
    .execute(&desk.db.pool)
    .await
    .expect("insert a customer");
    let document = format!("DOC-FD3-{n}");
    sqlx::query_scalar(
        "INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, \
             customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
             bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
             value_date, paid_at, recorded_at, recorded_by) \
         VALUES ($1, 'MAIN', 'DEMO', $1, 'payload-hash', $2, 'Customer', 'THA', 'SELL', 'USD', \
             100, 32.6, 3260.00, 0, 0, 3260.00, 0, 'Cash', '', '', 'owner', '', '', \
             DATE '2026-10-06', now(), now(), 'owner') RETURNING id::text",
    )
    .bind(&document)
    .bind(&customer)
    .fetch_one(&desk.db.pool)
    .await
    .expect("insert an exchange")
}

/// A feed row for exchange `id`. `updated_at` and the other times are RFC 3339 text.
pub struct Row<'a> {
    pub id: &'a str,
    pub customer_sub: Option<&'a str>,
    pub phone: Option<&'a str>,
    pub status: &'a str,
    pub value_satang: i64,
    pub reversed_satang: i64,
    pub completed_at: &'a str,
    pub reversed_at: Option<&'a str>,
    pub updated_at: &'a str,
}

pub async fn feed_row(desk: &Desk, row: &Row<'_>) {
    sqlx::query(
        "INSERT INTO broker_exchange_feed (transaction_id, organization_id, branch_id, \
             customer_sub, phone, status, value_satang, reversed_satang, completed_at, \
             reversed_at, updated_at) \
         SELECT $1::uuid, b.organization_id, 'MAIN', $2, $3, $4, $5, $6, $7::timestamptz, \
             $8::timestamptz, $9::timestamptz FROM branches b WHERE b.id = 'MAIN'",
    )
    .bind(row.id)
    .bind(row.customer_sub)
    .bind(row.phone)
    .bind(row.status)
    .bind(row.value_satang)
    .bind(row.reversed_satang)
    .bind(row.completed_at)
    .bind(row.reversed_at)
    .bind(row.updated_at)
    .execute(&desk.db.pool)
    .await
    .expect("insert a feed row");
}

/// A COMPLETED row of exchange `id` last changed at `updated_at`.
pub async fn completed(desk: &Desk, id: &str, updated_at: &str) {
    feed_row(
        desk,
        &Row {
            id,
            customer_sub: None,
            phone: None,
            status: "COMPLETED",
            value_satang: 326_000,
            reversed_satang: 0,
            completed_at: updated_at,
            reversed_at: None,
            updated_at,
        },
    )
    .await;
}

pub fn ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .map(|i| i["id"].as_str().expect("id is a string").to_string())
        .collect()
}
