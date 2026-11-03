//! `my_shift.incomingTransfers` (design `docs/design/blueprint/cash-shift-transfer.md` §4.7, "`my_shift.incomingTransfers`
//! (added 2026-11-02)", restated 2026-11-03 "the teller sees every transfer addressed to their shift", rows M-1…M-10,
//! and "Every transfer row names its creator", rows N-1…N-4): the teller sees every transfer into their own open shift,
//! whatever its status, read-only; receiving stays DISPATCHED-only. Another shift's transfer never shows. Every
//! `CashTransferRow` carries `createdByName` (null for a missing or blank name, never the id).
//!
//! Root cause pinned here: the only transfer list, `cash_transfers`, needs `CashManage`
//! (`xc-auth/src/domain/policy/areas/cash.rs:108-112`), so a teller (`ShiftOperate`) could receive a transfer it could
//! not see (#564 issuecomment-6572949093).
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests in
//! a file of your own.
//!
//! Setup: MAIN, two TELLERs, each with an OPEN shift opened through the cash-count routes (as
//! `cash_rounding_transaction_test.rs` does); the transfers are written straight into `cash_transfers` (setup only:
//! the view under test only reads them). Everything read goes over the wire as the teller.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

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
    teller_a: Who,
    teller_b: Who,
    shift_a: String,
    shift_b: String,
}

impl Room {
    async fn start() -> Self {
        let db = TestDb::start().await;
        db.exec(
            "INSERT INTO ops_state (branch_id, phase) VALUES ('MAIN', 'OPEN') \
             ON CONFLICT (branch_id) DO UPDATE SET phase = 'OPEN'",
        )
        .await;
        // The safe holds THB 1,000 for the two opening counts to draw on.
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
        let teller_a = Self::seed_person(&db, Role::Teller).await;
        let teller_b = Self::seed_person(&db, Role::Teller).await;
        let mut room = Self {
            db,
            manager,
            teller_a,
            teller_b,
            shift_a: String::new(),
            shift_b: String::new(),
        };
        room.shift_a = room.open_shift(&room.teller_a).await;
        room.shift_b = room.open_shift(&room.teller_b).await;
        assert_ne!(room.shift_a, room.shift_b);
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

    /// Opens `teller`'s shift with THB 20 × 1 through the cash-count routes (the manager verifies).
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

    /// Writes a transfer of THB 20 × `qty` (setup only) and answers its id. `to_shift` / `from_shift` set the ends.
    async fn transfer(
        &self,
        kind: &str,
        status: &str,
        from_shift: Option<&str>,
        to_shift: Option<&str>,
        qty: i32,
        created_at_offset_secs: i64,
    ) -> String {
        let id: String = sqlx::query_scalar(
            "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, from_shift_id, to_shift_id, status, \
                 created_by, idempotency_key, created_at, dispatched_at, dispatched_by) \
             VALUES ($1::uuid, $2, 'MAIN', $3, $4, $5, $6::uuid, $7, now() + make_interval(secs => $8), \
                 CASE WHEN $5 IN ('DISPATCHED', 'RECEIVED', 'RECONCILED') THEN now() END, \
                 CASE WHEN $5 IN ('DISPATCHED', 'RECEIVED', 'RECONCILED') THEN $6::uuid END) \
             RETURNING id::text",
        )
        .bind(ORG_1)
        .bind(kind)
        .bind(from_shift)
        .bind(to_shift)
        .bind(status)
        .bind(&self.manager.id)
        .bind(unique("incoming"))
        .bind(created_at_offset_secs as f64)
        .fetch_one(&self.db.pool)
        .await
        .unwrap_or_else(|e| panic!("write a {status} {kind} transfer: {e}"));
        sqlx::query(
            "INSERT INTO cash_transfer_lines (transfer_id, organization_id, currency, denomination_id, qty_sent) \
             VALUES ($1::uuid, $2::uuid, 'THB', $3, $4)",
        )
        .bind(&id)
        .bind(ORG_1)
        .bind(self.thb20().await)
        .bind(qty)
        .execute(&self.db.pool)
        .await
        .expect("its line");
        id
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

    /// `my_shift` as `who`: the view's `data`.
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

fn ids_of(incoming: &Value) -> Vec<String> {
    incoming
        .as_array()
        .unwrap_or_else(|| panic!("incomingTransfers is an array: {incoming}"))
        .iter()
        .map(|r| r["id"].as_str().expect("an id").to_owned())
        .collect()
}

/// Row 1: the teller's own DISPATCHED SAFE_TO_DRAWER transfer is listed, as a `cash_transfers` row with its lines.
#[actix_web::test]
async fn a_teller_sees_the_dispatched_transfer_into_their_own_shift() {
    let room = Room::start().await;
    let t = room
        .transfer(
            "SAFE_TO_DRAWER",
            "DISPATCHED",
            None,
            Some(&room.shift_a),
            5,
            0,
        )
        .await;

    let data = room.my_shift(&room.teller_a).await;
    assert_eq!(data["id"], json!(room.shift_a));
    let incoming = &data["incomingTransfers"];
    assert_eq!(ids_of(incoming), vec![t.clone()]);
    let row = &incoming[0];
    assert_eq!(row["kind"], json!("SAFE_TO_DRAWER"));
    assert_eq!(row["status"], json!("DISPATCHED"));
    assert_eq!(row["fromBranchId"], json!("MAIN"));
    assert_eq!(row["toShiftId"], json!(room.shift_a));
    assert_eq!(row["fromShiftId"], Value::Null);
    assert_eq!(row["createdBy"], json!(room.manager.id));
    assert_eq!(row["lines"].as_array().map(Vec::len), Some(1), "{row}");
    assert_eq!(row["lines"][0]["currency"], json!("THB"));
    assert_eq!(row["lines"][0]["qtySent"], json!("5"));
    assert_eq!(row["lines"][0]["qtyReceived"], Value::Null);
    room.finish().await;
}

/// Row 3: another teller's transfer, in the same branch, never shows; each teller sees only their own.
#[actix_web::test]
async fn another_shifts_transfer_never_shows() {
    let room = Room::start().await;
    let to_a = room
        .transfer(
            "SAFE_TO_DRAWER",
            "DISPATCHED",
            None,
            Some(&room.shift_a),
            5,
            0,
        )
        .await;
    let to_b = room
        .transfer(
            "SAFE_TO_DRAWER",
            "DISPATCHED",
            None,
            Some(&room.shift_b),
            7,
            0,
        )
        .await;

    assert_eq!(
        ids_of(&room.my_shift(&room.teller_a).await["incomingTransfers"]),
        vec![to_a]
    );
    assert_eq!(
        ids_of(&room.my_shift(&room.teller_b).await["incomingTransfers"]),
        vec![to_b]
    );
    room.finish().await;
}

/// Row 2, restated 2026-11-03 (M-1, M-4, M-5, M-6, M-7): every status into the shift is listed, oldest first.
/// Was `only_a_dispatched_transfer_is_listed` (DISPATCHED only), reversed by the lane controller's ruling of 2026-11-03.
#[actix_web::test]
async fn every_status_into_the_shift_is_listed_oldest_first() {
    let room = Room::start().await;
    let mut expected = Vec::new();
    for (offset, status) in ["DRAFT", "APPROVED", "DISPATCHED", "RECEIVED", "RECONCILED"]
        .into_iter()
        .enumerate()
    {
        expected.push(
            room.transfer(
                "SAFE_TO_DRAWER",
                status,
                None,
                Some(&room.shift_a),
                1,
                offset as i64 * 10,
            )
            .await,
        );
    }

    let data = room.my_shift(&room.teller_a).await;
    let incoming = &data["incomingTransfers"];
    assert_eq!(ids_of(incoming), expected, "{data}");
    let statuses: Vec<&str> = incoming
        .as_array()
        .expect("an array")
        .iter()
        .map(|r| r["status"].as_str().expect("a status"))
        .collect();
    assert_eq!(
        statuses,
        vec!["DRAFT", "APPROVED", "DISPATCHED", "RECEIVED", "RECONCILED"]
    );
    room.finish().await;
}

/// Row 4: a transfer leaving the shift (DRAWER_TO_SAFE, `from_shift_id` = it) is not incoming.
#[actix_web::test]
async fn a_transfer_leaving_the_shift_is_not_listed() {
    let room = Room::start().await;
    room.transfer(
        "DRAWER_TO_SAFE",
        "DISPATCHED",
        Some(&room.shift_a),
        None,
        1,
        0,
    )
    .await;

    let data = room.my_shift(&room.teller_a).await;
    assert_eq!(data["incomingTransfers"], json!([]), "{data}");
    room.finish().await;
}

/// No transfer at all: an empty list, never absent.
#[actix_web::test]
async fn with_nothing_incoming_the_list_is_empty() {
    let room = Room::start().await;
    let data = room.my_shift(&room.teller_a).await;
    assert_eq!(data["incomingTransfers"], json!([]), "{data}");
    room.finish().await;
}

/// Several incoming transfers are listed oldest first, the order they should be received in.
#[actix_web::test]
async fn several_incoming_transfers_are_listed_oldest_first() {
    let room = Room::start().await;
    let later = room
        .transfer(
            "SAFE_TO_DRAWER",
            "DISPATCHED",
            None,
            Some(&room.shift_a),
            1,
            60,
        )
        .await;
    let earlier = room
        .transfer(
            "SAFE_TO_DRAWER",
            "DISPATCHED",
            None,
            Some(&room.shift_a),
            2,
            0,
        )
        .await;

    assert_eq!(
        ids_of(&room.my_shift(&room.teller_a).await["incomingTransfers"]),
        vec![earlier, later]
    );
    room.finish().await;
}

/// Row 5: a caller with no open shift still gets `null`, as today.
#[actix_web::test]
async fn a_caller_with_no_open_shift_gets_null() {
    let room = Room::start().await;
    room.transfer(
        "SAFE_TO_DRAWER",
        "DISPATCHED",
        None,
        Some(&room.shift_a),
        5,
        0,
    )
    .await;
    assert_eq!(room.my_shift(&room.manager).await, Value::Null);
    room.finish().await;
}

impl Room {
    /// Attaches an approval request in `status` to transfer `id` (setup only), as `transferSubmit` would.
    async fn approval(&self, id: &str, status: &str) {
        sqlx::query(
            "WITH r AS ( \
                 INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
                     object_version_hash, policy_version, proposed_payload, requested_by, requested_at, status, \
                     idempotency_key) \
                 VALUES (gen_random_uuid(), $1::uuid, 'CASH_TRANSFER', 'MAIN', $2, 'h', '1', '{}'::jsonb, $3, now(), \
                     $4, $5) \
                 RETURNING id) \
             UPDATE cash_transfers SET approval_request_id = (SELECT id FROM r) WHERE id = $2::uuid",
        )
        .bind(ORG_1)
        .bind(id)
        .bind(&self.manager.id)
        .bind(status)
        .bind(unique("approval"))
        .execute(&self.db.pool)
        .await
        .unwrap_or_else(|e| panic!("attach a {status} approval: {e}"));
    }

    /// Sets `who`'s stored name (setup only).
    async fn name(&self, who: &Who, name: &str) {
        sqlx::query("UPDATE users SET name = $2 WHERE id = $1::uuid")
            .bind(&who.id)
            .bind(name)
            .execute(&self.db.pool)
            .await
            .expect("set the name");
    }

    /// Makes `who` the creator of transfer `id` (setup only).
    async fn created_by(&self, id: &str, who: &Who) {
        sqlx::query("UPDATE cash_transfers SET created_by = $2::uuid WHERE id = $1::uuid")
            .bind(id)
            .bind(&who.id)
            .execute(&self.db.pool)
            .await
            .expect("set the creator");
    }

    /// The `cash_transfers` view as `who`: the view's `data`.
    async fn cash_transfers(&self, who: &Who) -> Value {
        let a = self
            .call(
                who,
                Method::GET,
                "/api/v3/views/cash_transfers?branch=MAIN",
                None,
            )
            .await;
        assert_eq!(a.status, StatusCode::OK, "cash_transfers: {}", a.text);
        a.body["data"].clone()
    }
}

fn row_of<'a>(rows: &'a Value, id: &str) -> &'a Value {
    rows.as_array()
        .unwrap_or_else(|| panic!("an array of rows: {rows}"))
        .iter()
        .find(|r| r["id"] == json!(id))
        .unwrap_or_else(|| panic!("row {id} is listed: {rows}"))
}

/// M-1: a DRAFT never submitted is listed with no approval.
#[actix_web::test]
async fn m1_a_draft_never_submitted_is_listed_without_approval() {
    let room = Room::start().await;
    let t = room
        .transfer("SAFE_TO_DRAWER", "DRAFT", None, Some(&room.shift_a), 1, 0)
        .await;

    let data = room.my_shift(&room.teller_a).await;
    let row = row_of(&data["incomingTransfers"], &t);
    assert_eq!(row["status"], json!("DRAFT"), "{row}");
    assert_eq!(row["approvalStatus"], Value::Null, "{row}");
    assert_eq!(row["approvalRequestId"], Value::Null, "{row}");
    room.finish().await;
}

/// M-2 (and §4.10's T-7 on `my_shift`): a DRAFT waiting for approval is listed as PENDING.
#[actix_web::test]
async fn m2_a_pending_transfer_is_listed_as_pending() {
    let room = Room::start().await;
    let t = room
        .transfer("SAFE_TO_DRAWER", "DRAFT", None, Some(&room.shift_a), 1, 0)
        .await;
    room.approval(&t, "PENDING").await;

    let data = room.my_shift(&room.teller_a).await;
    let row = row_of(&data["incomingTransfers"], &t);
    assert_eq!(row["status"], json!("DRAFT"), "{row}");
    assert_eq!(row["approvalStatus"], json!("PENDING"), "{row}");
    room.finish().await;
}

/// M-3: a rejected transfer stays DRAFT and is listed as REJECTED.
#[actix_web::test]
async fn m3_a_rejected_transfer_is_listed_as_rejected() {
    let room = Room::start().await;
    let t = room
        .transfer("SAFE_TO_DRAWER", "DRAFT", None, Some(&room.shift_a), 1, 0)
        .await;
    room.approval(&t, "REJECTED").await;

    let data = room.my_shift(&room.teller_a).await;
    let row = row_of(&data["incomingTransfers"], &t);
    assert_eq!(row["status"], json!("DRAFT"), "{row}");
    assert_eq!(row["approvalStatus"], json!("REJECTED"), "{row}");
    room.finish().await;
}

/// M-8: another teller's shift is never listed, whatever the status.
#[actix_web::test]
async fn m8_another_shifts_transfer_is_never_listed_in_any_status() {
    let room = Room::start().await;
    for status in ["DRAFT", "APPROVED", "DISPATCHED", "RECEIVED", "RECONCILED"] {
        room.transfer("SAFE_TO_DRAWER", status, None, Some(&room.shift_b), 1, 0)
            .await;
    }
    let data = room.my_shift(&room.teller_a).await;
    assert_eq!(data["incomingTransfers"], json!([]), "{data}");
    room.finish().await;
}

/// M-10: the teller cannot receive a transfer that is visible but not dispatched; nothing changes.
#[actix_web::test]
async fn m10_receiving_a_listed_transfer_that_is_not_dispatched_is_refused() {
    let room = Room::start().await;
    let thb20 = room.thb20().await;
    let pending = room
        .transfer("SAFE_TO_DRAWER", "DRAFT", None, Some(&room.shift_a), 1, 0)
        .await;
    room.approval(&pending, "PENDING").await;
    let approved = room
        .transfer(
            "SAFE_TO_DRAWER",
            "APPROVED",
            None,
            Some(&room.shift_a),
            1,
            10,
        )
        .await;

    for t in [&pending, &approved] {
        let a = room
            .call(
                &room.teller_a,
                Method::POST,
                &format!("/api/cash-transfers/{t}/receive"),
                Some(json!({
                    "branch": "MAIN",
                    "lines": [{ "currency": "THB", "denominationId": thb20, "qtyReceived": "1" }]
                })),
            )
            .await;
        assert!(
            a.status.is_client_error(),
            "receive of {t} must be refused: {} {}",
            a.status,
            a.text
        );
    }
    let data = room.my_shift(&room.teller_a).await;
    let incoming = &data["incomingTransfers"];
    assert_eq!(row_of(incoming, &pending)["status"], json!("DRAFT"));
    assert_eq!(row_of(incoming, &approved)["status"], json!("APPROVED"));
    room.finish().await;
}

/// N-1: a `cash_transfers` row names its creator.
#[actix_web::test]
async fn n1_a_cash_transfers_row_names_its_creator() {
    let room = Room::start().await;
    room.name(&room.manager, "Malee Manager").await;
    let t = room
        .transfer(
            "SAFE_TO_DRAWER",
            "DISPATCHED",
            None,
            Some(&room.shift_a),
            1,
            0,
        )
        .await;

    let data = room.cash_transfers(&room.manager).await;
    let row = row_of(&data, &t);
    assert_eq!(row["createdBy"], json!(room.manager.id), "{row}");
    assert_eq!(row["createdByName"], json!("Malee Manager"), "{row}");
    room.finish().await;
}

/// N-2: a `my_shift.incomingTransfers` row names its creator.
#[actix_web::test]
async fn n2_a_my_shift_row_names_its_creator() {
    let room = Room::start().await;
    room.name(&room.manager, "Malee Manager").await;
    let t = room
        .transfer("SAFE_TO_DRAWER", "DRAFT", None, Some(&room.shift_a), 1, 0)
        .await;

    let data = room.my_shift(&room.teller_a).await;
    let row = row_of(&data["incomingTransfers"], &t);
    assert_eq!(row["createdByName"], json!("Malee Manager"), "{row}");
    room.finish().await;
}

/// N-3: a blank name answers null, never the id, on both views.
#[actix_web::test]
async fn n3_a_blank_name_is_null_never_the_id() {
    let room = Room::start().await;
    room.name(&room.manager, "   ").await;
    let t = room
        .transfer(
            "SAFE_TO_DRAWER",
            "DISPATCHED",
            None,
            Some(&room.shift_a),
            1,
            0,
        )
        .await;

    let shift = room.my_shift(&room.teller_a).await;
    assert_eq!(
        row_of(&shift["incomingTransfers"], &t)["createdByName"],
        Value::Null,
        "{shift}"
    );
    let list = room.cash_transfers(&room.manager).await;
    assert_eq!(row_of(&list, &t)["createdByName"], Value::Null, "{list}");
    room.finish().await;
}

/// N-4: rows by two creators each name their own creator.
#[actix_web::test]
async fn n4_each_row_names_its_own_creator() {
    let room = Room::start().await;
    room.name(&room.manager, "Malee Manager").await;
    room.name(&room.teller_b, "Somchai Teller").await;
    let by_manager = room
        .transfer("SAFE_TO_DRAWER", "DRAFT", None, Some(&room.shift_a), 1, 0)
        .await;
    let by_teller = room
        .transfer("SAFE_TO_DRAWER", "DRAFT", None, Some(&room.shift_a), 1, 10)
        .await;
    room.created_by(&by_teller, &room.teller_b).await;

    let data = room.my_shift(&room.teller_a).await;
    let incoming = &data["incomingTransfers"];
    assert_eq!(
        row_of(incoming, &by_manager)["createdByName"],
        json!("Malee Manager")
    );
    assert_eq!(
        row_of(incoming, &by_teller)["createdByName"],
        json!("Somchai Teller")
    );
    room.finish().await;
}
