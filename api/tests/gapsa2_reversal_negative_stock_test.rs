//! XC-BE-GAPS-A row 2 — a reversal post short of stock opens a `NEGATIVE_STOCK` approval request, and posts once the
//! branch's BRANCH_MANAGER approves it.
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 2, 2026-10-27). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Decided, and pinned here:
//! - `docs/design/blueprint/reversal.md:151` (§4.3, the opposite posting): "A shortfall is `stockInsufficient`, or
//!   needs an approved `NEGATIVE_STOCK` (#553 §5.1)". The post therefore answers as the completion does
//!   (`s2_completion_negative_stock_test.rs`, #3405): 409 `approvalRequired` with one PENDING request; APPROVED ⇒ the
//!   retry posts with `allow_negative`; after a rejection, plain `stockInsufficient` noting it and no new request.
//! - `ledger-inventory.md:418-425` (§5.1): `may_decide` is the BRANCH_MANAGER of the request's branch, so the request's
//!   `branch_id` is the poster's branch (MAIN).
//! - `reversal.md` §4.3 (last paragraph) and `ledger_postings.rs` `allows_negative`: the reversal's posting has
//!   `source_type REVERSAL` and `source_id` = the reversal id, and `allows_negative` compares the request's `object_id`
//!   with that `source_id` — so the request names the reversal (`NegativeStockObject::Reversal`), never the original
//!   exchange.
//! - Workflow level: `PostReversal::run_in` with no approved request still refuses `stockInsufficient`
//!   (`rv3_reversal_post_test.rs`, `a_shortfall_in_the_drawer_is_stock_insufficient_and_writes_nothing`); with an
//!   APPROVED request for the reversal it posts below zero; an approval naming the original exchange does not count.
//!
//! Guards (they pass on the skeleton on purpose): `gapsa2_a_post_within_the_drawers_stock_opens_no_request` and
//! `gapsa2_run_in_ignores_an_approval_naming_the_original_exchange`.
//!
//! The branch is TX-5's `Counter` (MAIN, its open shift with USD 1,000.00 in the ledger drawer). The original is a LIVE
//! cash BUY of USD 100 completed through the real `LedgerCashPostings` (the drawer then holds USD 1,100.00); every USD
//! is then moved out to the bank, so reversing the BUY (USD 100 drawer → customer) takes the drawer to USD −100.00.
//! The reversal is requested and FINANCE-approved through RV-2's workflow, as `reversal_post_route_test.rs` does. The
//! poster is MAIN's open shift's own teller (#2964), signed in as `own-teller`; the decider is MAIN's `MANAGER`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use pos_error::AppError;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{self, Role};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::reversal::reversal::ReversalId;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionId};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xchange::application::workflows::exchange::complete::{CompleteExchange, ExchangeEffects};
use xchange::application::workflows::exchange::negative_stock_gate::{
    NegativeStockGate, NegativeStockObject, NegativeStockOutcome,
};
use xchange::application::workflows::exchange::reversal_approval;
use xchange::application::workflows::exchange::reversal_post::PostReversal;
use xchange::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};
use xchange::application::workflows::save_step::SaveActor;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::commands::make_approval_service;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
/// The poster: a teller whose session carries a `users.id`, and who holds MAIN's open shift (#2964).
const OWN_TELLER: &str = "00000000-0000-0000-0000-0000000ba502";
const OWN: &str = "own-teller";
const MANAGER_AS: &str = "staff:MANAGER:MAIN";

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn finance() -> access::Actor {
    access::Actor::new(
        "finance-MAIN",
        "00000000-0000-0000-0000-000000000001",
        vec![access::ScopedRole::new(
            Role::Finance,
            Some(BranchId::new("MAIN")),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

/// The poster as an `Actor` (the requester of the `NEGATIVE_STOCK` request), as `own-teller` signs in.
fn poster_actor() -> access::Actor {
    test_callers::staff(OWN_TELLER, &[Role::Teller], BranchId::new("MAIN"))
}

/// MAIN's branch manager, who may decide a `NEGATIVE_STOCK` request of MAIN (§5.1), as `staff:MANAGER:MAIN`.
fn branch_manager() -> access::Actor {
    test_callers::staff(
        "employee-MANAGER",
        &[Role::BranchManager],
        BranchId::new("MAIN"),
    )
}

/// The poster as the exchange workflows are told who acts.
fn poster() -> SaveActor {
    SaveActor {
        id: OWN_TELLER.to_owned(),
        username: "own-teller".to_owned(),
        employee_id: None,
        standing: access::StandingKind::Staff(Role::Teller),
    }
}

/// Who is signed in, from `x-test-as`: `own-teller` (FRONT of MAIN, by `users.id`) or `staff:<ROLE>:<branch>`.
struct HeaderCallers;

fn staff_roles(role: &str) -> &'static [Role] {
    match role {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "FINANCE" => &[Role::Finance],
        other => panic!("unknown staff role {other:?}"),
    }
}

fn callers_of(spec: &str) -> Callers {
    let staff = match spec {
        "" => None,
        "own-teller" => Some(test_callers::staff(
            OWN_TELLER,
            &[Role::Teller],
            BranchId::new("MAIN"),
        )),
        other => {
            let mut parts = other.splitn(3, ':');
            assert_eq!(parts.next(), Some("staff"), "staff spec {other:?}");
            let role = parts.next().expect("a role");
            let branch = parts.next().expect("a branch");
            Some(test_callers::staff(
                &format!("employee-{role}"),
                staff_roles(role),
                BranchId::new(branch),
            ))
        }
    };
    test_callers::callers(None, staff)
}

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
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

    fn message(&self) -> &str {
        self.body["error"]["message"].as_str().unwrap_or("")
    }
}

struct Shop {
    c: Counter,
    approvals: Arc<ApprovalService>,
    requests: RequestReversal,
}

impl Shop {
    async fn start() -> Shop {
        let c = Counter::start().await;
        let mut registry = ApprovalRegistry::new();
        reversal_approval::register(&mut registry, c.desk.db.pool.clone());
        let approvals = Arc::new(ApprovalService::new(
            registry,
            Arc::new(PgApprovalRepository::new(c.desk.db.pool.clone())),
            Arc::new(PgAuditLog::new(c.desk.db.pool.clone())),
        ));
        let requests = RequestReversal::new(c.desk.db.pool.clone(), approvals.clone());
        let s = Shop {
            c,
            approvals,
            requests,
        };
        s.own_the_open_shift().await;
        s
    }

    async fn finish(self) {
        self.c.finish().await;
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.c.desk.db.pool
    }

    /// #2964: the poster exists as a `users` row and MAIN's open shift becomes theirs.
    async fn own_the_open_shift(&self) {
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             SELECT $1::uuid, id, 'own teller', 'ACTIVE', 'test' FROM organizations WHERE code = 'XC' \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(OWN_TELLER)
        .execute(self.pool())
        .await
        .expect("seed the poster");
        sqlx::query("UPDATE shifts SET teller_id = $1::uuid WHERE id = $2")
            .bind(OWN_TELLER)
            .bind(&self.c.shift)
            .execute(self.pool())
            .await
            .expect("re-point MAIN's open shift");
    }

    /// A LIVE cash BUY of USD `qty` completed at `now` + 5 s through TX-5 with the ledger's port.
    async fn completed_buy(&self, qty: &str, now: DateTime<Utc>) -> ExchangeTransaction {
        let q = self.c.desk.quote("MAIN", Side::Buy, qty, None, now).await;
        let t = self.c.live_of(&q, PaymentMethod::Cash, true, now).await;
        let service = CompleteExchange::new(
            self.pool().clone(),
            self.c.files.clone(),
            Arc::new(LedgerCashPostings::new()),
            Vec::new(),
            Arc::new(ExchangeEffects::new(Vec::new())),
        );
        let mut tx = self.pool().begin().await.unwrap();
        service
            .run_in(&mut tx, t.id, &teller(), later(now, 5))
            .await
            .unwrap_or_else(|e| panic!("complete the original: {e:?}"));
        tx.commit().await.unwrap();
        self.c.desk.stored(t.id).await
    }

    /// RV-2's request at `now` + 10 s, approved by MAIN's FINANCE.
    async fn approved(&self, t: &ExchangeTransaction, now: DateTime<Utc>) -> ReversalId {
        let mut tx = self.pool().begin().await.unwrap();
        let r = self
            .requests
            .run_in(
                &mut tx,
                RequestReversalInput {
                    branch: BranchId::new("MAIN"),
                    original: t.id,
                    reason: "the customer changed their mind".to_owned(),
                    key: next("gapsa2-request"),
                },
                &teller(),
                later(now, 10),
            )
            .await
            .unwrap_or_else(|e| panic!("request the reversal: {e:?}"));
        tx.commit().await.unwrap();
        let mut tx = self.pool().begin().await.unwrap();
        self.approvals
            .decide(
                &mut tx,
                &RequestId::new(r.approval_request_id.to_string()),
                &finance(),
                Outcome::Approved,
                "checked the drawer",
                &next("gapsa2-decide"),
            )
            .await
            .unwrap_or_else(|e| panic!("FINANCE approves: {}", e.code()));
        tx.commit().await.unwrap();
        r.id
    }

    /// An APPROVED reversal of a USD 100 cash BUY whose post would take the drawer to USD −100.00.
    async fn short_reversal(&self) -> (ExchangeTransaction, ReversalId) {
        let now = whole_second_now();
        let t = self.completed_buy("100", now).await;
        let id = self.approved(&t, now).await;
        let usd = self.drawer("USD").await;
        assert_eq!(usd, d("1100"), "the drawer holds the bought USD");
        self.out_to_bank(&usd.to_string()).await;
        assert_eq!(
            self.drawer("USD").await,
            Decimal::ZERO,
            "the drawer is empty"
        );
        (t, id)
    }

    /// `q` USD out of MAIN's open drawer to the bank (a TRANSFER posting).
    async fn out_to_bank(&self, q: &str) {
        let source = SourceRef {
            source_type: SourceType::Transfer,
            source_id: next("gapsa2-out"),
            source_key: "dispatch".to_owned(),
        };
        let reqs = vec![PostingRequest {
            from: CashPlace::Drawer(self.c.shift.clone()),
            to: CashPlace::Bank,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(d(q)),
            source,
            line: None,
            reverses: None,
        }];
        let mut tx = self.pool().begin().await.unwrap();
        LedgerCashPostings::new()
            .post_in(&mut tx, &reqs, None)
            .await
            .unwrap_or_else(|e| panic!("empty the drawer: {e:?}"));
        tx.commit().await.unwrap();
    }

    /// MAIN's open drawer's balance of `currency`, read from the signed ledger sum (it may be below zero).
    async fn drawer(&self, currency: &str) -> Decimal {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(e.quantity_signed), 0) FROM ledger_entries e \
             JOIN cash_accounts a ON a.id = e.account_id \
             WHERE a.kind = 'DRAWER' AND a.shift_id = $1 AND trim(a.currency) = $2",
        )
        .bind(&self.c.shift)
        .bind(currency)
        .fetch_one(self.pool())
        .await
        .expect("the drawer's signed ledger sum")
    }

    async fn int(&self, sql: &str) -> i64 {
        self.c.desk.db.count(sql).await
    }

    async fn status(&self, id: ReversalId) -> String {
        self.c
            .desk
            .db
            .text(&format!(
                "SELECT status::text FROM reversals WHERE id = '{}'",
                id.0
            ))
            .await
            .unwrap_or_default()
    }

    async fn reversal_postings(&self, id: ReversalId) -> i64 {
        self.int(&format!(
            "SELECT count(*) FROM ledger_postings WHERE source_type = 'REVERSAL' AND source_id = '{}'",
            id.0
        ))
        .await
    }

    async fn reversal_receipts(&self, t: TransactionId) -> i64 {
        self.int(&format!(
            "SELECT count(*) FROM receipts WHERE transaction_id = '{}' AND revision = 2",
            t.0
        ))
        .await
    }

    /// The `NEGATIVE_STOCK` requests naming `object_id`, as `(id, status, branch_id)` in the order they were opened.
    async fn requests_for(&self, object_id: &str) -> Vec<(String, String, Option<String>)> {
        sqlx::query_as(
            "SELECT id::text, status, branch_id FROM approval_requests \
             WHERE type = 'NEGATIVE_STOCK' AND object_id = $1 ORDER BY requested_at, id",
        )
        .bind(object_id)
        .fetch_all(self.pool())
        .await
        .expect("read approval_requests")
    }

    async fn all_negative_stock(&self) -> i64 {
        self.int("SELECT count(*) FROM approval_requests WHERE type = 'NEGATIVE_STOCK'")
            .await
    }

    /// The reversal is still APPROVED: nothing posted, no revision-2 receipt, the drawer still empty of USD.
    async fn assert_not_posted(&self, t: &ExchangeTransaction, id: ReversalId) {
        assert_eq!(
            self.status(id).await,
            "APPROVED",
            "the reversal stays APPROVED"
        );
        assert_eq!(self.reversal_postings(id).await, 0, "no reversal posting");
        assert_eq!(
            self.reversal_receipts(t.id).await,
            0,
            "no revision-2 receipt"
        );
        assert_eq!(
            self.drawer("USD").await,
            Decimal::ZERO,
            "the drawer's USD did not move"
        );
    }

    /// `PostReversal::run_in` as the command runs it: committed on `Ok`, rolled back on `Err`.
    async fn run_in(&self, id: ReversalId) -> Result<(), AppError> {
        let service = PostReversal::new(
            self.pool().clone(),
            self.c.files.clone(),
            Arc::new(LedgerCashPostings::new()),
            Arc::new(ExchangeEffects::new(Vec::new())),
        );
        let mut tx = self.pool().begin().await.unwrap();
        match service.run_in(&mut tx, id, &poster(), Utc::now()).await {
            Ok(_) => {
                tx.commit().await.unwrap();
                Ok(())
            }
            Err(e) => {
                tx.rollback().await.unwrap();
                Err(e)
            }
        }
    }

    /// Opens a `NEGATIVE_STOCK` request for `object` through the gate and has MAIN's branch manager approve it.
    async fn approve_negative_stock(&self, object: &NegativeStockObject) -> String {
        let approvals = make_approval_service(self.pool().clone());
        let opened = NegativeStockGate::open_after_refusal(
            self.pool(),
            &approvals,
            object,
            &BranchId::new("MAIN"),
            "stockInsufficient: the posting takes a drawer, a safe or a position below zero",
            &poster_actor(),
            &next("gapsa2-ns"),
        )
        .await
        .unwrap_or_else(|e| panic!("open the NEGATIVE_STOCK request: {e:?}"));
        let NegativeStockOutcome::ApprovalRequired(req) = opened else {
            panic!("a fresh request is PENDING: {opened:?}");
        };
        let mut tx = self.pool().begin().await.unwrap();
        approvals
            .decide(
                &mut tx,
                &RequestId::new(req.as_str().to_owned()),
                &branch_manager(),
                Outcome::Approved,
                "XC-BE-GAPS-A row 2 up-front test",
                &next("gapsa2-ns-decide"),
            )
            .await
            .unwrap_or_else(|e| panic!("MAIN's branch manager approves: {}", e.code()));
        tx.commit().await.unwrap();
        req.as_str().to_owned()
    }
}

/// `POST path` as `who` with `key` as the `Idempotency-Key`.
async fn post(s: &Shop, path: &str, who: &str, body: Value, key: &str) -> Answer {
    let dir = s.c.scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        s.pool().clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(path)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, who))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// The poster posts reversal `id` with `key`.
async fn post_reversal(s: &Shop, id: ReversalId, key: &str) -> Answer {
    post(
        s,
        &format!("/api/reversals/{}/post", id.0),
        OWN,
        json!({}),
        key,
    )
    .await
}

/// MAIN's branch manager decides `request_id` (`APPROVED` or `REJECTED`) over the wire; it must be accepted.
async fn decide(s: &Shop, request_id: &str, outcome: &str) {
    let a = post(
        s,
        &format!("/api/approvals/{request_id}/decide"),
        MANAGER_AS,
        json!({ "outcome": outcome, "reason": "XC-BE-GAPS-A row 2 up-front test" }),
        &unique("decide"),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "MAIN's branch manager decides {outcome}: {}",
        a.text
    );
}

// ------------------------------------------------------------ a shortfall opens one PENDING request

#[actix_web::test]
async fn gapsa2_a_post_beyond_the_drawer_opens_one_pending_request_naming_the_reversal() {
    let s = Shop::start().await;
    let (t, id) = s.short_reversal().await;

    let a = post_reversal(&s, id, &unique("key")).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::CONFLICT, "approvalRequired"),
        "reversal.md:151 / §5.1: the desk is told to wait for approval: {}",
        a.text
    );
    let rows = s.requests_for(&id.0.to_string()).await;
    assert_eq!(
        rows.len(),
        1,
        "one NEGATIVE_STOCK request names the reversal (the posting's source_id): {rows:?}"
    );
    assert_eq!(rows[0].1, "PENDING");
    assert_eq!(
        rows[0].2.as_deref(),
        Some("MAIN"),
        "the request is the poster's branch's, so MAIN's manager may decide it"
    );
    assert_eq!(
        s.requests_for(&t.id.0.to_string()).await.len(),
        0,
        "no request names the original exchange: allows_negative would never match it"
    );
    assert_eq!(s.all_negative_stock().await, 1, "one request in all");
    s.assert_not_posted(&t, id).await;
    s.finish().await;
}

// ------------------------------------------------------------ no second request while one is pending

#[actix_web::test]
async fn gapsa2_the_same_key_again_opens_no_second_request() {
    let s = Shop::start().await;
    let (t, id) = s.short_reversal().await;
    let key = unique("key");

    let first = post_reversal(&s, id, &key).await;
    assert_eq!(first.code(), "approvalRequired", "{}", first.text);
    let again = post_reversal(&s, id, &key).await;
    assert_eq!(
        (again.status, again.code()),
        (StatusCode::CONFLICT, "approvalRequired"),
        "{}",
        again.text
    );
    let rows = s.requests_for(&id.0.to_string()).await;
    assert_eq!(rows.len(), 1, "still one request: {rows:?}");
    assert_eq!(rows[0].1, "PENDING");
    s.assert_not_posted(&t, id).await;
    s.finish().await;
}

#[actix_web::test]
async fn gapsa2_a_new_key_while_a_request_is_pending_opens_no_second_request() {
    let s = Shop::start().await;
    let (t, id) = s.short_reversal().await;

    let first = post_reversal(&s, id, &unique("key")).await;
    assert_eq!(first.code(), "approvalRequired", "{}", first.text);
    let other = post_reversal(&s, id, &unique("key")).await;
    assert_eq!(
        (other.status, other.code()),
        (StatusCode::CONFLICT, "approvalRequired"),
        "AlreadyPending answers the same as the first: {}",
        other.text
    );
    let rows = s.requests_for(&id.0.to_string()).await;
    assert_eq!(rows.len(), 1, "still one request: {rows:?}");
    assert_eq!(rows[0].1, "PENDING");
    assert_eq!(s.all_negative_stock().await, 1, "one request in all");
    s.assert_not_posted(&t, id).await;
    s.finish().await;
}

// ------------------------------------------------------------ APPROVED ⇒ the retry posts below zero

#[actix_web::test]
async fn gapsa2_once_the_branch_manager_approves_the_retry_posts_and_the_drawer_goes_below_zero() {
    let s = Shop::start().await;
    let (t, id) = s.short_reversal().await;

    let first = post_reversal(&s, id, &unique("key")).await;
    assert_eq!(first.code(), "approvalRequired", "{}", first.text);
    let rows = s.requests_for(&id.0.to_string()).await;
    assert_eq!(rows.len(), 1, "one request to decide: {rows:?}");
    decide(&s, &rows[0].0, "APPROVED").await;

    let retry = post_reversal(&s, id, &unique("key")).await;
    assert_eq!(
        retry.status,
        StatusCode::OK,
        "§5.1: the retry posts with allow_negative: {}",
        retry.text
    );
    assert_eq!(retry.body["reversal"]["status"], "POSTED", "{}", retry.text);
    assert_eq!(s.status(id).await, "POSTED");
    assert_eq!(s.reversal_postings(id).await, 1, "one reversal posting");
    assert_eq!(s.reversal_receipts(t.id).await, 1, "the reversal document");
    assert_eq!(
        s.drawer("USD").await,
        d("-100"),
        "the drawer goes below zero by the shortfall (0 − 100)"
    );
    assert_eq!(
        s.requests_for(&id.0.to_string()).await.len(),
        1,
        "the retry opened no other request"
    );
    s.finish().await;
}

// ------------------------------------------------------------ after a rejection

#[actix_web::test]
async fn gapsa2_after_a_rejection_the_retry_is_stock_insufficient_noting_it_and_no_new_request() {
    let s = Shop::start().await;
    let (t, id) = s.short_reversal().await;
    let key = unique("key");

    let first = post_reversal(&s, id, &key).await;
    assert_eq!(first.code(), "approvalRequired", "{}", first.text);
    let rows = s.requests_for(&id.0.to_string()).await;
    assert_eq!(rows.len(), 1, "one request to decide: {rows:?}");
    decide(&s, &rows[0].0, "REJECTED").await;

    let after = post_reversal(&s, id, &key).await;
    assert_eq!(
        (after.status, after.code()),
        (StatusCode::CONFLICT, "stockInsufficient"),
        "the owner (#564): plain stockInsufficient after a rejection: {}",
        after.text
    );
    assert!(
        after.message().to_lowercase().contains("reject"),
        "the answer notes the rejection: {:?}",
        after.message()
    );
    let rows = s.requests_for(&id.0.to_string()).await;
    assert_eq!(rows.len(), 1, "no new request: {rows:?}");
    assert_eq!(
        rows[0].1, "REJECTED",
        "the rejected request is never reopened"
    );
    s.assert_not_posted(&t, id).await;
    s.finish().await;
}

// ------------------------------------------------------------ enough stock asks nobody (guard)

#[actix_web::test]
async fn gapsa2_a_post_within_the_drawers_stock_opens_no_request() {
    let s = Shop::start().await;
    let now = whole_second_now();
    let t = s.completed_buy("100", now).await;
    let id = s.approved(&t, now).await;

    let a = post_reversal(&s, id, &unique("key")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["reversal"]["status"], "POSTED", "{}", a.text);
    assert_eq!(s.drawer("USD").await, d("1000"), "the USD 100 went back");
    assert_eq!(
        s.all_negative_stock().await,
        0,
        "a post within stock opens no NEGATIVE_STOCK request"
    );
    s.finish().await;
}

// ------------------------------------------------------------ workflow level

/// `PostReversal::run_in` looks up the APPROVED `NEGATIVE_STOCK` request for the reversal and passes it to the port.
#[actix_web::test]
async fn gapsa2_run_in_posts_below_zero_with_an_approved_request_for_the_reversal() {
    let s = Shop::start().await;
    let (t, id) = s.short_reversal().await;

    let refused = s.run_in(id).await.expect_err("no approval yet");
    assert!(
        matches!(&refused, AppError::Domain(m) if m.starts_with("stockInsufficient")),
        "with no approval the workflow still refuses stockInsufficient: {refused:?}"
    );
    s.approve_negative_stock(&NegativeStockObject::Reversal(id))
        .await;

    s.run_in(id).await.unwrap_or_else(|e| {
        panic!("an approved NEGATIVE_STOCK for the reversal lets it post: {e:?}")
    });
    assert_eq!(s.status(id).await, "POSTED");
    assert_eq!(s.reversal_postings(id).await, 1);
    assert_eq!(s.reversal_receipts(t.id).await, 1);
    assert_eq!(s.drawer("USD").await, d("-100"));
    s.finish().await;
}

/// Guard: an approval naming the original exchange is not the reversal's (`allows_negative` compares `object_id`
/// with the posting's `source_id`, the reversal id).
#[actix_web::test]
async fn gapsa2_run_in_ignores_an_approval_naming_the_original_exchange() {
    let s = Shop::start().await;
    let (t, id) = s.short_reversal().await;

    s.approve_negative_stock(&NegativeStockObject::Transaction(t.id))
        .await;
    let refused = s
        .run_in(id)
        .await
        .expect_err("the approval is not this source's");
    assert!(
        matches!(&refused, AppError::Domain(m) if m.starts_with("stockInsufficient")),
        "{refused:?}"
    );
    s.assert_not_posted(&t, id).await;
    s.finish().await;
}
