//! LA-7 up-front test (design `docs/design/blueprint/ledger-inventory.md` §13.5 "Read", §13.9 row LA-T24):
//! `GET /api/v3/views/ledger_adjustments?branch=&status=`.
//!
//! 🔴 Written by the sub-controller claude/f32b85db before the implementation. Do not edit, delete or weaken;
//! add tests in a file of your own. The harness is `ledger_adjustment_common/mod.rs`. The POSTED adjustment's
//! posting is written through the port directly (`differences` under `(ADJUSTMENT, id, "post")`), so this file
//! needs neither command.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

#![allow(dead_code)]

mod common;
mod ledger_adjustment_common;

use actix_web::http::{Method, StatusCode};
use ledger_adjustment_common::*;
use serde_json::{Value, json};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, differences};
use xc_kernel::ids::BranchId;

const VIEW: &str = "/api/v3/views/ledger_adjustments";

async fn read(room: &Room, who: &str, branch: &str, status: Option<&str>) -> Answer {
    let uri = match status {
        Some(s) => format!("{VIEW}?branch={branch}&status={s}"),
        None => format!("{VIEW}?branch={branch}"),
    };
    call(room, who, Method::GET, &uri, None, "").await
}

fn rows(a: &Answer) -> Vec<Value> {
    a.body["data"]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("data.rows is an array: {}", a.text))
        .clone()
}

fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("a row for {id}: {rows:?}"))
}

#[actix_web::test]
async fn la_t24_the_branch_lists_its_adjustments_with_their_status() {
    let room = Room::start().await;
    let pending = room
        .adjustment("PENDING", quantity_payload(B1, &[("USD", "-10")]))
        .await;
    let approved = room
        .adjustment("APPROVED", quantity_payload(B1, &[("USD", "-20")]))
        .await;
    let rejected = room
        .adjustment("REJECTED", quantity_payload(B1, &[("USD", "-30")]))
        .await;
    let posted = room
        .adjustment("APPROVED", quantity_payload(B1, &[("USD", "-100")]))
        .await;
    room.post_seed(differences(
        &CashPlace::Safe(BranchId::new(B1)),
        &CashPlace::Variance(BranchId::new(B1)),
        &[(usd(), d("-100"))],
        &SourceRef {
            source_type: SourceType::Adjustment,
            source_id: posted.clone(),
            source_key: "post".to_owned(),
        },
    ))
    .await;
    // XC-B02's adjustment, stored (as every request of an organisation-wide FINANCE) under another branch_id.
    let mut other = quantity_payload(B2, &[("USD", "-1")]);
    other["evidence"] = json!({ "files": [FILE_B2], "reference": "" });
    let foreign = room.adjustment("PENDING", other).await;

    let a = read(&room, FIN_A, B1, None).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let all = rows(&a);
    assert_eq!(all.len(), 4, "XC-B01's four, not XC-B02's: {}", a.text);
    assert!(
        all.iter().all(|r| r["id"] != foreign.as_str()),
        "XC-B02's adjustment is not listed even though its row's branch_id is XC-B01: {}",
        a.text
    );
    for (id, status) in [
        (&pending, "PENDING"),
        (&approved, "APPROVED"),
        (&rejected, "REJECTED"),
        (&posted, "POSTED"),
    ] {
        let r = row(&all, id);
        assert_eq!(r["status"], status, "{id}: {r}");
        assert_eq!(r["kind"], "QUANTITY", "{r}");
        assert_eq!(r["place"], "SAFE", "{r}");
        assert_eq!(
            r["reason"], "Bank deposit keyed as 1,100 USD; the slip says 1,000",
            "{r}"
        );
        assert_eq!(r["evidence"]["files"], json!([FILE_B1]), "{r}");
        assert_eq!(r["evidence"]["reference"], "slip 004512", "{r}");
        assert_eq!(r["requestedBy"], user_id(FIN_A).as_str(), "{r}");
        if status == "POSTED" {
            assert!(r["postingId"].is_string(), "{r}");
            assert!(r["postedAt"].is_string(), "{r}");
            assert_eq!(r["valuations"][0]["currency"], "USD", "{r}");
            assert_eq!(r["valuations"][0]["valuationThb"], "3250.00", "{r}");
        } else {
            assert!(r["postingId"].is_null(), "postingId on POSTED only: {r}");
            assert_eq!(r["valuations"], json!([]), "{r}");
        }
        if status == "PENDING" {
            assert!(r["decidedBy"].is_null(), "{r}");
        } else {
            assert_eq!(r["decidedBy"], user_id(FIN_B).as_str(), "{r}");
        }
    }

    let only_posted = read(&room, FIN_A, B1, Some("POSTED")).await;
    assert_eq!(only_posted.status, StatusCode::OK, "{}", only_posted.text);
    let ids: Vec<Value> = rows(&only_posted).iter().map(|r| r["id"].clone()).collect();
    assert_eq!(
        ids,
        vec![json!(posted)],
        "status=POSTED: {}",
        only_posted.text
    );

    let approved_only = read(&room, FIN_A, B1, Some("APPROVED")).await;
    let ids: Vec<Value> = rows(&approved_only)
        .iter()
        .map(|r| r["id"].clone())
        .collect();
    assert_eq!(
        ids,
        vec![json!(approved)],
        "APPROVED excludes the posted one: {}",
        approved_only.text
    );

    let bad = read(&room, FIN_A, B1, Some("DONE")).await;
    assert_refused(
        &bad,
        StatusCode::BAD_REQUEST,
        "invalidRequest",
        "status=DONE",
    );

    let teller = read(&room, TELLER, B1, None).await;
    assert!(
        teller.status.is_client_error(),
        "a TELLER is refused: {}",
        teller.text
    );
    assert!(teller.body["data"].is_null(), "no rows: {}", teller.text);
    room.finish().await;
}
