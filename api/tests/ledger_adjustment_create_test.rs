//! LA-5 up-front tests (design `docs/design/blueprint/ledger-inventory.md` §13.4, §13.5 "Create", §13.9 rows
//! LA-T11 to LA-T14): `POST /api/v3/commands/ledger_adjustment_create`.
//!
//! 🔴 Written by the sub-controller claude/f32b85db before the implementation. Do not edit, delete or weaken;
//! add tests in a file of your own. The harness is `ledger_adjustment_common/mod.rs`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

#![allow(dead_code)]

mod common;
mod ledger_adjustment_common;

use actix_web::http::StatusCode;
use ledger_adjustment_common::*;
use serde_json::json;

// ── LA-T11 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t11_finance_creates_a_requested_adjustment_and_nothing_is_posted() {
    let room = Room::start().await;
    let before = room.written().await;
    let a = create(
        &room,
        FIN_A,
        quantity_payload(B1, &[("USD", "-100")]),
        &unique("k"),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let id = a.body["id"].as_str().expect("an id").to_owned();
    assert!(id.starts_with("ADJ-"), "{}", a.text);
    assert!(
        uuid::Uuid::parse_str(&id["ADJ-".len()..]).is_ok(),
        "ADJ-<uuid>: {id}"
    );
    assert_eq!(a.body["status"], "REQUESTED", "{}", a.text);
    let request_id = a.body["approvalRequestId"]
        .as_str()
        .expect("approvalRequestId")
        .to_owned();

    let (stored_id, status, payload): (String, String, serde_json::Value) = sqlx::query_as(
        "SELECT id::text, status, proposed_payload FROM approval_requests \
          WHERE type = 'LEDGER_ADJUSTMENT' AND object_id = $1",
    )
    .bind(&id)
    .fetch_one(&room.pool)
    .await
    .expect("exactly one LEDGER_ADJUSTMENT request");
    assert_eq!(stored_id, request_id);
    assert_eq!(status, "PENDING");
    assert_eq!(payload["branch"], B1, "{payload}");
    assert_eq!(payload["kind"], "QUANTITY", "{payload}");
    assert_eq!(payload["place"], "SAFE", "{payload}");
    assert_eq!(payload["lines"][0]["currency"], "USD", "{payload}");
    assert_eq!(payload["lines"][0]["quantity"], "-100", "{payload}");
    assert_eq!(payload["evidence"]["files"][0], FILE_B1, "{payload}");

    assert_eq!(room.postings_of(&id).await, 0, "no posting at create");
    let (postings, entries, movements, lots, requests) = room.written().await;
    assert_eq!(
        (postings, entries, movements, lots, requests),
        (before.0, before.1, before.2, before.3, before.4 + 1),
        "only the request is written"
    );
    assert_eq!(room.position(B1, "USD").await, Some((d("1000"), d("32500.00"))));

    let audits = room.audit_rows(CREATE_AUDIT).await;
    assert_eq!(audits.len(), 1, "one LEDGER_ADJUSTMENT_CREATE row: {audits:?}");
    let (actor, object_ref, branch, _after) = &audits[0];
    assert_eq!(actor, &user_id(FIN_A));
    assert_eq!(object_ref, &format!("ledger_adjustment:{id}"));
    assert_eq!(branch.as_deref(), Some(B1));
    room.finish().await;
}

// ── LA-T12 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t12_a_manager_or_a_teller_cannot_create() {
    let room = Room::start().await;
    for who in [MGR, TELLER] {
        let a = create(
            &room,
            who,
            quantity_payload(B1, &[("USD", "-100")]),
            &unique("k"),
        )
        .await;
        // The seam's answer for a role the policy row refuses: `400 roleNotAllowedHere` (#4551; §13.9 says
        // 403, measured 2026-10-28 on the skeleton, where the refusal comes before `run`).
        assert_refused(
            &a,
            StatusCode::BAD_REQUEST,
            "roleNotAllowedHere",
            &format!("{who} is refused by the policy row (FinanceReconcile)"),
        );
    }
    assert_eq!(
        room.count("SELECT count(*) FROM approval_requests").await,
        0,
        "nothing written"
    );
    assert_eq!(room.audits(CREATE_AUDIT).await, 0);
    room.finish().await;
}

// ── LA-T13 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t13_an_evidence_file_of_another_branch_or_unknown_is_refused() {
    let room = Room::start().await;
    for (file, what) in [(FILE_B2, "a file of XC-B02"), ("NO-SUCH-FILE", "an unknown file")] {
        let mut body = quantity_payload(B1, &[("USD", "-100")]);
        body["evidence"] = json!({ "files": [file], "reference": "" });
        let a = create(&room, FIN_A, body, &unique("k")).await;
        assert_refused(&a, StatusCode::BAD_REQUEST, "validationFailed", what);
        assert_eq!(field(&a), "evidence.files", "{what}: {}", a.text);
    }
    assert_eq!(
        room.count("SELECT count(*) FROM approval_requests").await,
        0,
        "nothing written"
    );
    assert_eq!(room.audits(CREATE_AUDIT).await, 0);
    room.finish().await;
}

/// §13.4 refusals that reach the wire: one per field the command must name.
#[actix_web::test]
async fn la_t13_payload_refusals_name_their_field() {
    let room = Room::start().await;
    let mut no_evidence = quantity_payload(B1, &[("USD", "-100")]);
    no_evidence["evidence"] = json!({ "files": [], "reference": "  " });
    let mut drawer = quantity_payload(B1, &[("USD", "-100")]);
    drawer["place"] = json!("DRAWER");
    let mut unknown_exception = quantity_payload(B1, &[("USD", "-100")]);
    unknown_exception["exceptionId"] = json!("EX-nope");
    for (body, want, what) in [
        (no_evidence, "evidence", "no file and a blank reference"),
        (drawer, "place", "place DRAWER"),
        (
            quantity_payload(B1, &[("USD", "0")]),
            "lines[0].quantity",
            "quantity 0",
        ),
        (unknown_exception, "exceptionId", "an unknown exception"),
    ] {
        let a = create(&room, FIN_A, body, &unique("k")).await;
        assert_refused(&a, StatusCode::BAD_REQUEST, "validationFailed", what);
        assert_eq!(field(&a), want, "{what}: {}", a.text);
    }
    assert_eq!(
        room.count("SELECT count(*) FROM approval_requests").await,
        0,
        "nothing written"
    );
    room.finish().await;
}

// ── LA-T14 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn la_t14_a_repeated_key_replays_and_another_body_conflicts() {
    let room = Room::start().await;
    let key = unique("k");
    let body = quantity_payload(B1, &[("USD", "-100")]);
    let first = create(&room, FIN_A, body.clone(), &key).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let again = create(&room, FIN_A, body, &key).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.body, first.body, "the same answer");
    assert_eq!(
        room.count("SELECT count(*) FROM approval_requests WHERE type = 'LEDGER_ADJUSTMENT'")
            .await,
        1,
        "one request"
    );

    let other = create(&room, FIN_A, quantity_payload(B1, &[("USD", "-50")]), &key).await;
    assert_refused(
        &other,
        StatusCode::CONFLICT,
        "idempotencyConflict",
        "the same key with another body",
    );
    assert_eq!(
        room.count("SELECT count(*) FROM approval_requests WHERE type = 'LEDGER_ADJUSTMENT'")
            .await,
        1
    );
    assert_eq!(room.audits(CREATE_AUDIT).await, 1);
    room.finish().await;
}
