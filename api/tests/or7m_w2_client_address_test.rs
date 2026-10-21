//! OR-7.m w2 W2-T1 (#4014) — the caller's address comes from `X-Real-IP`, tested on the kiosk
//! quote limit (20 quotes a minute per address, the 21st refused with `400 TOO_MANY_QUOTES`).

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::{Counter, TOO_MANY_QUOTES, kiosk};
use rpc_common::Answer;
use serde_json::json;

const ALLOWED: usize = 20;
const CF: &str = "cf-connecting-ip";
const XFF: &str = "x-forwarded-for";
const REAL: &str = "x-real-ip";

async fn quote(counter: &Counter, headers: &[(&str, &str)]) -> Answer {
    let payload = json!({ "type": "BUY", "currency": "USD", "amount": 1 });
    let mut call = kiosk(&counter.origin, "quote", payload);
    for (name, value) in headers {
        call = call.header(name, value);
    }
    counter.app.send(call).await
}

/// Row 1: 21 quotes, one `X-Real-IP` `203.0.113.7`, each with a different `CF-Connecting-IP`
/// → the 21st is refused: a new CF value does not walk around the limit.
#[actix_web::test]
async fn row_1_counts_under_x_real_ip_whatever_cf_connecting_ip_says() {
    let counter = Counter::open(&[]).await;
    for n in 0..ALLOWED {
        let answer = quote(
            &counter,
            &[(CF, &format!("198.51.100.{n}")), (REAL, "203.0.113.7")],
        )
        .await;
        assert_eq!(answer.status, 200, "quote {}: {}", n + 1, answer.text);
    }
    let refused = quote(&counter, &[(CF, "198.51.100.250"), (REAL, "203.0.113.7")]).await;
    refused.assert_refused(
        400,
        TOO_MANY_QUOTES,
        "the 21st from one X-Real-IP is refused: a new CF-Connecting-IP on every request does not walk around the limit",
    );
    counter.finish().await;
}

/// Row 2: 21 quotes, one `CF-Connecting-IP` `198.51.100.9`, each with a different `X-Real-IP`
/// → none is refused: CF does not join callers.
#[actix_web::test]
async fn row_2_does_not_join_callers_that_send_one_cf_connecting_ip() {
    let counter = Counter::open(&[]).await;
    for n in 0..=ALLOWED {
        let answer = quote(
            &counter,
            &[(CF, "198.51.100.9"), (REAL, &format!("203.0.113.{n}"))],
        )
        .await;
        assert_eq!(
            answer.status,
            200,
            "quote {}: each X-Real-IP is a caller of its own, whatever CF-Connecting-IP says: {}",
            n + 1,
            answer.text
        );
    }
    counter.finish().await;
}

/// Row 3a: 21 quotes, one `X-Real-IP`, each with a different `X-Forwarded-For`
/// → the 21st is refused (XFF does not split).
#[actix_web::test]
async fn row_3a_x_forwarded_for_does_not_split_one_x_real_ip() {
    let counter = Counter::open(&[]).await;
    for n in 0..ALLOWED {
        let answer = quote(
            &counter,
            &[
                (XFF, &format!("198.51.100.{n}, 10.0.0.1")),
                (REAL, "203.0.113.1"),
            ],
        )
        .await;
        assert_eq!(answer.status, 200, "quote {}: {}", n + 1, answer.text);
    }
    let refused = quote(&counter, &[(XFF, "198.51.100.252"), (REAL, "203.0.113.1")]).await;
    refused.assert_refused(
        400,
        TOO_MANY_QUOTES,
        "the 21st from one X-Real-IP is refused: a new X-Forwarded-For on every request does not walk around the limit",
    );
    counter.finish().await;
}

/// Row 3b: 21 quotes, each with a different `X-Real-IP`, one `X-Forwarded-For`
/// → none is refused (XFF does not join).
#[actix_web::test]
async fn row_3b_x_forwarded_for_does_not_join_distinct_x_real_ips() {
    let counter = Counter::open(&[]).await;
    for n in 0..=ALLOWED {
        let answer = quote(
            &counter,
            &[(XFF, "198.51.100.252"), (REAL, &format!("203.0.113.{n}"))],
        )
        .await;
        assert_eq!(
            answer.status,
            200,
            "quote {}: X-Forwarded-For does not make one caller of distinct X-Real-IP values: {}",
            n + 1,
            answer.text
        );
    }
    counter.finish().await;
}

/// Row 4: 21 quotes without a usable `X-Real-IP`: mixed absent, `""` and `"   "`
/// → the 21st is refused: they share one counter (`UNKNOWN_ADDRESS`, `client_address.rs:22`).
#[actix_web::test]
async fn row_4_callers_without_a_usable_x_real_ip_share_one_counter() {
    let counter = Counter::open(&[]).await;
    let nameless = |n: usize| -> Vec<(&'static str, &'static str)> {
        match n % 3 {
            0 => vec![],
            1 => vec![(REAL, "")],
            _ => vec![(REAL, "   ")],
        }
    };
    for n in 0..ALLOWED {
        let headers = nameless(n);
        let answer = quote(&counter, &headers).await;
        assert_eq!(
            answer.status,
            200,
            "quote {} ({headers:?}): {}",
            n + 1,
            answer.text
        );
    }
    let refused = quote(&counter, &nameless(ALLOWED)).await;
    refused.assert_refused(
        400,
        TOO_MANY_QUOTES,
        "the 21st quote without a usable X-Real-IP is refused: absent, empty and blank share UNKNOWN_ADDRESS",
    );
    counter.finish().await;
}
