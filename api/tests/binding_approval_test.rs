//! Up-front tests for BQ-6 (the AP-1 subject `BINDING_QUOTE`; design `docs/design/blueprint/binding-quote.md`
//! D-15, D-19, §10; §4.2 Q22's decider part and Q27). Written by the sub-controller claude/644b3958 on
//! `feat/binding-quote` before the implementation. **Do not edit, delete or weaken.**
//!
//! No quote exists while a binding quote waits for approval, so the request names the **proposal**:
//! `binding_approval::proposal_id(req, coupon)` is a lowercase sha256 hex over the canonical issue request.
//! The proposal never changes, so `version_hash(object_id) == object_id` and `on_decided` writes nothing. The
//! decider is T2's: the holder of `ApproveExchangeTransaction` on a row covering the request's branch (the
//! BRANCH_MANAGER of that branch); anyone else is `outOfScope`, and AP-1 refuses the proposer itself.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`), as every `tx4_*` test does.

mod common;
mod quote_common;
mod tx4_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tx4_common::*;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::Actor;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xchange::application::workflows::exchange::binding_approval::{
    BINDING_QUOTE, BindingQuoteApprovalPolicy, BindingQuoteSubject, proposal_id,
};
use xchange::application::workflows::exchange::quote::{QuoteRequest, QuoteRequestLine};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MANAGER: &str = "employee-manager";

fn quantity(text: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(text.parse::<Decimal>().unwrap())
}

/// A DESK binding-quote request at MAIN: 100 USD sold for cash to `CUSTOMER`, no fee, group ALL.
fn base() -> QuoteRequest {
    QuoteRequest {
        branch_id: BranchId::new("MAIN"),
        customer_id: Some(CUSTOMER.to_owned()),
        channel: QuoteChannel::Desk,
        lines: vec![QuoteRequestLine {
            currency: CurrencyCode::parse("USD").unwrap(),
            shop_side: Side::Sell,
            quantity: quantity("100"),
            fee_thb: None,
            denomination_group: GroupCode::all(),
        }],
        payment_method: Some(PaymentMethod::Cash),
    }
}

fn changed(f: impl FnOnce(&mut QuoteRequest)) -> QuoteRequest {
    let mut r = base();
    f(&mut r);
    r
}

#[test]
fn proposal_id_is_a_lowercase_sha256_hex_and_the_same_for_the_same_request() {
    let id = proposal_id(&base(), None);
    assert_eq!(id.len(), 64, "{id}");
    assert!(
        id.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "{id}"
    );
    assert_eq!(proposal_id(&base(), None), id);
    assert_eq!(
        proposal_id(&base(), Some("SAVE10")),
        proposal_id(&base(), Some("SAVE10"))
    );
}

#[test]
fn proposal_id_changes_when_any_one_field_changes() {
    let cases: Vec<(&str, String)> = vec![
        ("base", proposal_id(&base(), None)),
        (
            "branch",
            proposal_id(&changed(|r| r.branch_id = BranchId::new("BR2")), None),
        ),
        (
            "customer",
            proposal_id(
                &changed(|r| r.customer_id = Some("CU-OTHER".to_owned())),
                None,
            ),
        ),
        (
            "no customer",
            proposal_id(&changed(|r| r.customer_id = None), None),
        ),
        (
            "channel",
            proposal_id(&changed(|r| r.channel = QuoteChannel::App), None),
        ),
        (
            "payment method",
            proposal_id(
                &changed(|r| r.payment_method = Some(PaymentMethod::BankTransfer)),
                None,
            ),
        ),
        (
            "no payment method",
            proposal_id(&changed(|r| r.payment_method = None), None),
        ),
        ("coupon", proposal_id(&base(), Some("SAVE10"))),
        (
            "currency",
            proposal_id(
                &changed(|r| r.lines[0].currency = CurrencyCode::parse("JPY").unwrap()),
                None,
            ),
        ),
        (
            "side",
            proposal_id(&changed(|r| r.lines[0].shop_side = Side::Buy), None),
        ),
        (
            "quantity",
            proposal_id(&changed(|r| r.lines[0].quantity = quantity("101")), None),
        ),
        (
            "fee",
            proposal_id(
                &changed(|r| r.lines[0].fee_thb = Some(money("10.00"))),
                None,
            ),
        ),
        (
            "group",
            proposal_id(
                &changed(|r| r.lines[0].denomination_group = GroupCode::new("LARGE").unwrap()),
                None,
            ),
        ),
        (
            "a second line",
            proposal_id(
                &changed(|r| {
                    let line = r.lines[0].clone();
                    r.lines.push(line);
                }),
                None,
            ),
        ),
    ];
    for (i, (a, ida)) in cases.iter().enumerate() {
        for (b, idb) in &cases[i + 1..] {
            assert_ne!(ida, idb, "{a} and {b} must be different proposals");
        }
    }
}

#[test]
fn proposal_id_does_not_change_with_how_the_quantity_is_written() {
    assert_eq!(
        proposal_id(&changed(|r| r.lines[0].quantity = quantity("100.00")), None),
        proposal_id(&base(), None),
        "100 and 100.00 are the same quantity"
    );
}

#[test]
fn the_policy_version_is_1() {
    assert_eq!(BindingQuoteApprovalPolicy.policy_version(), "1");
}

#[test]
fn the_subjects_type_is_binding_quote() {
    assert_eq!(BINDING_QUOTE, "BINDING_QUOTE");
    assert_eq!(BindingQuoteSubject.type_name(), BINDING_QUOTE);
}

#[tokio::test]
async fn the_generated_registry_knows_binding_quote() {
    let db = TestDb::start().await;
    let registry = bootstrap::approval_subjects::registry(db.pool.clone());
    let (subject, policy) = registry
        .get(BINDING_QUOTE)
        .expect("D-19: BINDING_QUOTE is registered from approval_subjects/binding_quote.rs");
    assert_eq!(subject.type_name(), BINDING_QUOTE);
    assert_eq!(policy.policy_version(), "1");
    db.finish().await;
}

#[tokio::test]
async fn the_version_hash_is_the_object_id() {
    let db = TestDb::start().await;
    let id = proposal_id(&base(), None);
    let mut tx = db.pool.begin().await.unwrap();
    let hash = BindingQuoteSubject
        .version_hash(&mut tx, &id)
        .await
        .unwrap_or_else(|e| panic!("version_hash: {e}"));
    tx.commit().await.unwrap();
    assert_eq!(hash, id);
    db.finish().await;
}

/// Rows a decision could touch.
async fn row_counts(db: &TestDb) -> [i64; 5] {
    [
        db.count("SELECT count(*) FROM quotes").await,
        db.count("SELECT count(*) FROM quote_lines").await,
        db.count("SELECT count(*) FROM stock_reservations").await,
        db.count("SELECT count(*) FROM transactions").await,
        db.count("SELECT count(*) FROM approval_requests").await,
    ]
}

#[tokio::test]
async fn on_decided_writes_nothing_whatever_the_outcome() {
    let db = TestDb::start().await;
    let id = proposal_id(&base(), None);
    let before = row_counts(&db).await;
    for outcome in [Outcome::Approved, Outcome::Rejected] {
        let mut tx = db.pool.begin().await.unwrap();
        BindingQuoteSubject
            .on_decided(&mut tx, &id, outcome)
            .await
            .unwrap_or_else(|e| panic!("on_decided({outcome:?}): {e}"));
        tx.commit().await.unwrap();
        assert_eq!(row_counts(&db).await, before, "{outcome:?}");
    }
    db.finish().await;
}

/// A PENDING `BINDING_QUOTE` request for the base proposal, opened through AP-1 by `by`.
async fn open(desk: &Desk, by: &Actor) -> RequestId {
    let mut tx = desk.db.pool.begin().await.unwrap();
    let request = desk
        .approvals
        .request(
            &mut tx,
            BINDING_QUOTE,
            &proposal_id(&base(), None),
            &json!({ "settleThb": "3245.00", "approvalLimitThb": "1000.00" }),
            by,
            &format!("bindingQuote:{}", uuid::Uuid::new_v4()),
        )
        .await
        .unwrap_or_else(|e| panic!("request: {e}"));
    tx.commit().await.unwrap();
    request.id
}

async fn request_status(desk: &Desk, request: &RequestId) -> String {
    desk.db
        .text(&format!(
            "SELECT status FROM approval_requests WHERE id = '{}'",
            request.as_str()
        ))
        .await
        .expect("a request row")
}

fn teller_of_main() -> Actor {
    staff_as(TELLER, "FRONT", "MAIN")
}

#[tokio::test]
async fn the_request_names_the_proposal_and_its_hash() {
    let desk = Desk::start().await;
    let request = open(&desk, &teller_of_main()).await;
    let id = proposal_id(&base(), None);
    assert_eq!(
        desk.db
            .text(&format!(
                "SELECT type || '|' || object_id || '|' || object_version_hash || '|' || \
                        COALESCE(branch_id, '') || '|' || status \
                 FROM approval_requests WHERE id = '{}'",
                request.as_str()
            ))
            .await
            .expect("a request row"),
        format!("BINDING_QUOTE|{id}|{id}|MAIN|PENDING")
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q22_the_proposer_cannot_decide_their_own_request_even_as_the_branchs_manager() {
    let desk = Desk::start().await;
    let manager = staff_as(MANAGER, "MANAGER", "MAIN");
    let request = open(&desk, &manager).await;

    assert_eq!(
        approval_code(
            desk.decide(&request, &manager, Outcome::Approved, "mine")
                .await
        ),
        "outOfScope"
    );
    assert_eq!(request_status(&desk, &request).await, "PENDING");
    desk.db.finish().await;
}

#[tokio::test]
async fn q22_a_teller_of_the_branch_cannot_decide() {
    let desk = Desk::start().await;
    let request = open(&desk, &teller_of_main()).await;

    assert_eq!(
        approval_code(
            desk.decide(
                &request,
                &staff_as("employee-other-teller", "FRONT", "MAIN"),
                Outcome::Approved,
                "ok"
            )
            .await
        ),
        "outOfScope"
    );
    assert_eq!(request_status(&desk, &request).await, "PENDING");
    desk.db.finish().await;
}

#[tokio::test]
async fn q22_a_manager_of_another_branch_cannot_decide() {
    let desk = Desk::start().await;
    let request = open(&desk, &teller_of_main()).await;

    assert_eq!(
        approval_code(
            desk.decide(
                &request,
                &staff_as("employee-br2-manager", "MANAGER", BR2),
                Outcome::Approved,
                "ok"
            )
            .await
        ),
        "outOfScope"
    );
    assert_eq!(request_status(&desk, &request).await, "PENDING");
    desk.db.finish().await;
}

#[tokio::test]
async fn q22_the_owner_does_not_decide_a_binding_quote() {
    let desk = Desk::start().await;
    let request = open(&desk, &teller_of_main()).await;

    assert_eq!(
        approval_code(
            desk.decide(&request, &owner("owner"), Outcome::Approved, "ok")
                .await
        ),
        "outOfScope",
        "T2 names the branch's manager, not the EXECUTIVE"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q22_the_branchs_manager_approves_and_nothing_else_is_written() {
    let desk = Desk::start().await;
    let request = open(&desk, &teller_of_main()).await;
    let before = row_counts(&desk.db).await;

    desk.decide(
        &request,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        Outcome::Approved,
        "known customer",
    )
    .await
    .expect("T2: the branch's manager approves");

    assert_eq!(request_status(&desk, &request).await, "APPROVED");
    assert_eq!(
        row_counts(&desk.db).await,
        before,
        "no quote until the retry"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q22_the_branchs_manager_rejects() {
    let desk = Desk::start().await;
    let request = open(&desk, &teller_of_main()).await;

    desk.decide(
        &request,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        Outcome::Rejected,
        "too large today",
    )
    .await
    .expect("the branch's manager rejects");

    assert_eq!(request_status(&desk, &request).await, "REJECTED");
    desk.db.finish().await;
}

// ---------------------------------------------------------------- Q27: the approval_requests view

const AS: &str = "x-test-as";

struct HeaderCallers;

/// `staff:<V2 ROLE>:<branch>`, as `approvals_read_per_type_test.rs`.
fn callers_of(spec: &str) -> Callers {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "spec {spec:?}");
    let role_name = parts.next().expect("a role");
    let roles = test_callers::v2_staff_roles(role_name)
        .unwrap_or_else(|| panic!("unknown role {role_name:?}"));
    let branch = parts.next().expect("a branch");
    test_callers::callers(
        None,
        Some(test_callers::staff(
            &format!("employee-{role_name}"),
            roles,
            BranchId::new(branch),
        )),
    )
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

/// The listed requests' `type`s; panics unless the view answers 200.
async fn listed(db: &TestDb, who: &str) -> Vec<String> {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri("/api/v3/views/approval_requests")
        .insert_header((header::HOST, "xc.test"))
        .insert_header((AS, who))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "{who}: {text}");
    let body: Value = serde_json::from_slice(&bytes).expect("JSON");
    body.pointer("/data/requests")
        .or_else(|| body.pointer("/requests"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{who}: no requests list in {text}"))
        .iter()
        .map(|r| r["type"].as_str().expect("a type").to_owned())
        .collect()
}

#[tokio::test]
async fn q27_a_pending_binding_quote_is_listed_for_the_manager_of_its_branch_only() {
    let desk = Desk::start().await;
    open(&desk, &teller_of_main()).await;

    assert_eq!(
        listed(&desk.db, "staff:MANAGER:MAIN").await,
        [BINDING_QUOTE],
        "D-19: MAIN's manager sees it, so can decide it"
    );
    assert_eq!(
        listed(&desk.db, &format!("staff:MANAGER:{BR2}")).await,
        Vec::<String>::new(),
        "another branch's manager does not"
    );
    desk.db.finish().await;
}
