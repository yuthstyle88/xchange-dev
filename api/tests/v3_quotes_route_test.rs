//! Ticket #669 (TX-1+TX-3), Part 2 (TX-3) — `POST /api/quotes` through the CM-1 command seam: the
//! `Idempotency-Key`, who may ask, the `QuoteDto` answer and that it is the stored quote, and the v3
//! refusal codes.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §4.2 (the route, permission
//! `ExchangeTransact` = today's `Policy` row until OR-4, the `QuoteDto` fields, the `[required]`
//! input `{ customerId?, channel, lines: [{ currency, shopSide, quantity, feeThb? }] }`, money and
//! quantities as decimal strings, the error codes), §4.3 (the server computes everything),
//! §9.1 TX-3; rates design `docs/design/blueprint/rates.md` §6 (the command seam: a missing
//! `Idempotency-Key` is `400`, the same key and body replays, the same key with another body is
//! `idempotencyConflict`). Ticket #669 Part 2 §7–§9.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Each test works in a schema of its own.
//!
//! Signatures this test assumes ("choice" marks a form the design leaves open, which the
//! implementer follows):
//! - `QuotesCommand: CommandAction` [required] at `POST /api/quotes`, registered through the
//!   commands table (`api/build.rs`), so `bootstrap::configure` serves it.
//! - choice: its `INHERITS` is `(EntryPath::Rpc, "saveTransaction")`, today's row for the desk's
//!   exchange (OWNER, and FRONT staff; `xc-auth/src/domain/policy/entities/policy.rs:78,85`).
//! - The body is the [required] input above. **The branch is the seam's, not the body's** (ticket
//!   author's ruling 4 on #669, 2026-10-09): the caller names it with `branch`, the key every v3
//!   command uses (`command.rs` step 5); with none named, the owner runs on MAIN and staff on their
//!   own branch (`Policy::branch`). The stored `quotes.branch_id` is that branch, and a `branchId`
//!   in the body is not read. A staff caller naming a branch other than its own is `403 outOfScope`
//!   and nothing is written. On main the seam's `Policy::branch` answers a staff caller's own branch
//!   whatever it names, so the refusal has to come from the command: for example `locks()` returning
//!   the named `branch` (the seam passes it into `Input`), which step 6.5 then refuses.
//! - `QuoteDto` [required], `xchange::presentation::dto::v3::quote::QuoteDto`, exported by ts-rs to
//!   `quote.ts`. The answer is the whole body (as CM-1 answers). choice: these keys, camelCase:
//!   `id` (the `quotes` row's uuid), `type`, `channel`, `expiresAt` (RFC 3339), `publication`
//!   `{ "id", "version" }`, `lines` `[{ currency, shopSide, quantity, unitSize, baseRate, finalRate,
//!   thbAmount, feeThb, settleThb, publishedRateLineId }]`, `benefits` (`[]` until #560) and `stale`
//!   (bool: the line was stale at read time and the policy admitted it). A line's side is
//!   `shopSide`, the input's key and the column's name. Money is a 2-place string (`"235.00"`);
//!   quantity, unit size and rates are decimal strings compared by value.
//! - Refusals answer the v3 body `{ "error": { "code", … } }`: `validationFailed` 400,
//!   `outOfScope` 403, `rateUnavailable` 409, `idempotencyConflict` 409 (the statuses of
//!   `commands/mod.rs::map_rate_engine_error` and of the seam).
//! - `created_by` is the signed-in actor's id (`Actor::id`).

mod common;
mod quote_common;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use ts_rs::TS;
use xc_auth::domain::access::{Actor as AccessActor, Role as AccessRole};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::dto::v3::quote::QuoteDto;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUTE: &str = "/api/quotes";
const FRONT: &str = "staff:FRONT:MAIN";

// ------------------------------------------------------------ callers

/// Who is signed in, from the request header `x-test-as`: `owner`, `staff:<ROLE>:<branch>`,
/// `owner+staff:<ROLE>:<branch>`, or absent.
struct HeaderCallers;

fn staff_caller_of(spec: &str) -> AccessActor {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role_name = parts.next().expect("a role");
    let roles: &[AccessRole] = match role_name {
        "FRONT" => &[AccessRole::Teller],
        "BACKEND" | "MANAGER" => &[AccessRole::BranchManager],
        "RATES" => &[AccessRole::RateManager],
        "FINANCE" => &[AccessRole::Finance],
        "SYSTEM" => &[AccessRole::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[AccessRole::VerificationReviewer],
        other => panic!("unknown role {other:?}"),
    };
    let branch = parts.next().expect("a branch");
    test_callers::staff(
        &format!("employee-{role_name}"),
        roles,
        BranchId::new(branch),
    )
}

fn callers_of(spec: &str) -> Callers {
    if spec.is_empty() {
        return test_callers::nobody();
    }
    if spec == "owner" {
        return test_callers::callers(Some(test_callers::owner("owner-1")), None);
    }
    if let Some(staff) = spec.strip_prefix("owner+") {
        return test_callers::callers(
            Some(test_callers::owner("owner-1")),
            Some(staff_caller_of(staff)),
        );
    }
    test_callers::callers(None, Some(staff_caller_of(spec)))
}

fn staff_of(spec: &str) -> Option<Actor> {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Actor::staff(
        format!("employee-{}", role.as_str()),
        role,
        BranchId::new(branch),
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

/// The actor the seam asks the policy about on Rpc (`view.rs:331`: the owner's session wins).
fn actor_on_rpc(spec: &str) -> Option<Actor> {
    if spec.is_empty() {
        return None;
    }
    if spec == "owner" || spec.starts_with("owner+") {
        return Some(Actor::owner("owner-1"));
    }
    staff_of(spec)
}

const STAFF_ROLES: [&str; 9] = [
    "FRONT",
    "BACKEND",
    "MANAGER",
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

/// The codes a policy refusal answers with (`V3Error::from(PolicyError)`).
const POLICY_CODES: [&str; 6] = [
    "signIn",
    "ownerOnly",
    "employeeAuthRequired",
    "invalidRequest",
    "roleNotAllowed",
    "roleNotAllowedHere",
];

// ------------------------------------------------------------ requests

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

/// `POST /api/quotes` as `who` with `body`, sending `key` as the `Idempotency-Key` when given.
async fn post(db: &TestDb, who: &str, body: &Value, key: Option<&str>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::post()
        .uri(ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(body.to_string());
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

/// `POST /api/quotes` with a fresh key.
async fn ask(db: &TestDb, who: &str, body: &Value) -> Answer {
    post(db, who, body, Some(&unique("key"))).await
}

/// A one-line DESK body naming no branch: the seam runs it on the caller's branch.
fn body(currency: &str, side: &str, quantity: &str, fee: Option<&str>) -> Value {
    let mut line = json!({ "currency": currency, "shopSide": side, "quantity": quantity });
    if let Some(fee) = fee {
        line["feeThb"] = json!(fee);
    }
    json!({ "channel": "DESK", "lines": [line] })
}

/// `b` naming `branch` with the seam's key, `branch` (`command.rs` step 5).
fn on(branch: &str, mut b: Value) -> Value {
    b["branch"] = json!(branch);
    b
}

fn dec(v: &Value, what: &str) -> Decimal {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a decimal string, got {v}"));
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{what} {s:?}: {e}"))
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

/// `answer` is refused with `code` and `status`, and nothing was stored.
async fn refused(db: &TestDb, a: &Answer, code: &str, status: StatusCode, what: &str) {
    assert_eq!(
        (a.status, a.code()),
        (status, code),
        "{what}: body {}",
        a.text
    );
    assert_eq!(db.quote_rows().await, (0, 0), "{what}: nothing stored");
}

// ------------------------------------------------------------ the answer

/// OR-4b (#767): the rule a route asks once it is on `Policy::authorize` (design §4.2): the caller
/// as B1 maps them (§3.3), naming `named` (D-4: a branch outside the caller's scope is refused).
fn blueprint_rule(
    actor: Option<&xc_auth::domain::policy::value_objects::actor::Actor>,
    path: xc_auth::domain::policy::value_objects::entry_path::EntryPath,
    action: &str,
    named: Option<&str>,
) -> Result<(), xc_auth::domain::policy::value_objects::policy_error::PolicyError> {
    use xc_auth::domain::policy::entities::policy::{Need, Policy};
    let act = xc_auth::domain::policy::value_objects::action::Action::new(action);
    if Policy::need(path, &act) == Some(Need::Custody) {
        // #582's rows stay on the v2 rule
        return Policy::allow(actor, path, &act);
    }
    let user = actor.map(|a| {
        xc_auth::domain::access::v3_roles::actor_from_v3(
            a,
            "XC",
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        )
    });
    Policy::authorize(
        user.as_ref(),
        path,
        &act,
        named.map(xc_kernel::ids::BranchId::new).as_ref(),
        chrono::Utc::now(),
    )
    .map(|_| ())
}

#[actix_web::test]
async fn the_answer_has_every_field_with_money_as_strings_and_is_the_stored_quote() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[JPY, USD]).await;

    let before = Utc::now();
    let a = ask(&db, FRONT, &body("JPY", "BUY", "1000", Some("5.00"))).await;
    let after = Utc::now();
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    let q = &a.body;

    // the quote
    let id = q["id"].as_str().unwrap_or_else(|| panic!("id: {}", a.text));
    let stored: Option<(String, String, String, String, DateTime<Utc>, String)> = sqlx::query_as(
        "SELECT type, channel, status, publication_id::text, expires_at, created_by \
         FROM quotes WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .expect("read quotes");
    let (ty, channel, status, publication, expires_at, created_by) =
        stored.unwrap_or_else(|| panic!("the answered id {id} is a row of quotes"));
    assert_eq!(
        (
            ty.as_str(),
            channel.as_str(),
            status.as_str(),
            created_by.as_str()
        ),
        ("INDICATIVE", "DESK", "ACTIVE", "employee-FRONT"),
        "the stored quote"
    );
    assert_eq!(q["type"], json!("INDICATIVE"), "type");
    assert_eq!(q["channel"], json!("DESK"), "channel");
    assert_eq!(
        q["publication"],
        json!({ "id": p.id, "version": p.version }),
        "the publication the quote was priced on"
    );
    assert_eq!(publication, p.id, "publication_id");
    let answered = DateTime::parse_from_rfc3339(
        q["expiresAt"]
            .as_str()
            .unwrap_or_else(|| panic!("expiresAt: {}", a.text)),
    )
    .expect("expiresAt is RFC 3339")
    .with_timezone(&Utc);
    assert!(
        (answered - expires_at).num_milliseconds().abs() < 1000,
        "expiresAt {answered} is the stored {expires_at}"
    );
    assert!(
        answered >= before + Duration::seconds(300) - Duration::seconds(1)
            && answered <= after + Duration::seconds(300) + Duration::seconds(1),
        "expiresAt {answered} is now + 300 s (between {before} and {after})"
    );
    assert_eq!(q["benefits"], json!([]), "no benefits until #560");
    assert_eq!(q["stale"], json!(false), "a MANUAL line is never stale");

    // the line
    let lines = q["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("lines: {}", a.text));
    assert_eq!(lines.len(), 1, "one line");
    let l = &lines[0];
    assert_eq!(l["currency"], json!("JPY"), "currency");
    assert_eq!(l["shopSide"], json!("BUY"), "shopSide");
    assert_eq!(dec(&l["quantity"], "quantity"), d("1000"));
    assert_eq!(dec(&l["unitSize"], "unitSize"), d("100"));
    assert_eq!(dec(&l["baseRate"], "baseRate"), d("23.5"));
    assert_eq!(dec(&l["finalRate"], "finalRate"), d("23.5"));
    assert_eq!(l["thbAmount"], json!("235.00"), "thbAmount");
    assert_eq!(l["feeThb"], json!("5.00"), "feeThb");
    assert_eq!(
        l["settleThb"],
        json!("230.00"),
        "settleThb: BUY = thb − fee"
    );
    assert_eq!(
        l["publishedRateLineId"],
        json!(p.line("JPY")),
        "the published line it was priced on"
    );
    let row: (String, String, String, String) = sqlx::query_as(
        "SELECT published_rate_line_id::text, thb_amount::text, fee_thb::text, settle_thb::text \
         FROM quote_lines WHERE quote_id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("the stored line");
    assert_eq!(
        row,
        (
            p.line("JPY").to_owned(),
            "235.00".to_owned(),
            "5.00".to_owned(),
            "230.00".to_owned()
        ),
        "the stored line is the answered one"
    );
    assert_eq!(db.quote_rows().await, (1, 1));
    db.finish().await;
}

#[actix_web::test]
async fn the_answer_follows_the_data_side_publication_and_staleness() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[JPY]).await;

    // SELL takes the sell rate and adds the fee
    let a = ask(&db, FRONT, &body("JPY", "SELL", "1000", Some("5.00"))).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    let l = &a.body["lines"][0];
    assert_eq!(
        (&l["shopSide"], &l["thbAmount"], &l["settleThb"]),
        (&json!("SELL"), &json!("240.00"), &json!("245.00")),
        "SELL JPY 1,000 at 24.0 per 100, fee 5"
    );

    // a new publication prices the next quote
    let second = db
        .publish("MAIN", &[Line::manual("JPY", "100", "22.0", "24.0")])
        .await;
    let a = ask(&db, FRONT, &body("JPY", "BUY", "1000", None)).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(
        (
            &a.body["lines"][0]["thbAmount"],
            &a.body["lines"][0]["publishedRateLineId"],
            &a.body["publication"]["id"],
        ),
        (
            &json!("220.00"),
            &json!(second.line("JPY")),
            &json!(second.id)
        ),
        "the new publication's line"
    );
    assert_eq!(a.body["lines"][0]["feeThb"], json!("0.00"), "no fee: 0.00");

    // a stale line the policy admits (on_stale NULL) is answered with stale: true
    let none = db.policy("MAIN", None, None).await;
    db.publish(
        "MAIN",
        &[Line {
            stale_under: Some(none),
            ..USD
        }],
    )
    .await;
    let a = ask(&db, FRONT, &body("USD", "BUY", "10", None)).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["stale"], json!(true), "stale at read time");
    assert_eq!(a.body["lines"][0]["thbAmount"], json!("321.00"));
    db.finish().await;
}

#[actix_web::test]
async fn the_customer_is_stored_when_named() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD]).await;
    db.customer("C-1", "MAIN").await;
    let mut b = body("USD", "BUY", "10", None);
    b["customerId"] = json!("C-1");
    let a = ask(&db, FRONT, &b).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    let id = a.body["id"].as_str().expect("id");
    assert_eq!(
        db.text(&format!("SELECT customer_id FROM quotes WHERE id = '{id}'"))
            .await
            .as_deref(),
        Some("C-1"),
        "customer_id"
    );
    db.finish().await;
}

// ------------------------------------------------------------ the command seam

#[actix_web::test]
async fn no_idempotency_key_is_400_and_stores_nothing() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[JPY]).await;
    let b = body("JPY", "BUY", "1000", None);
    let a = post(&db, FRONT, &b, None).await;
    refused(
        &db,
        &a,
        "validationFailed",
        StatusCode::BAD_REQUEST,
        "no key",
    )
    .await;
    let a = post(&db, FRONT, &b, Some("  ")).await;
    refused(
        &db,
        &a,
        "validationFailed",
        StatusCode::BAD_REQUEST,
        "a blank key",
    )
    .await;
    db.finish().await;
}

#[actix_web::test]
async fn the_same_key_and_body_replays_the_first_answer_and_stores_one_quote() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[JPY]).await;
    let key = unique("key");
    let b = body("JPY", "BUY", "1000", None);
    let first = post(&db, FRONT, &b, Some(&key)).await;
    assert_eq!(first.status, StatusCode::OK, "body {}", first.text);
    let again = post(&db, FRONT, &b, Some(&key)).await;
    assert_eq!(again.status, StatusCode::OK, "body {}", again.text);
    assert_eq!(again.body, first.body, "the replay is the first answer");
    assert_eq!(db.quote_rows().await, (1, 1), "one quote");

    // another key asks again
    let other = ask(&db, FRONT, &b).await;
    assert_eq!(other.status, StatusCode::OK, "body {}", other.text);
    assert_ne!(other.body["id"], first.body["id"], "a new quote");
    assert_eq!(db.quote_rows().await, (2, 2));
    db.finish().await;
}

#[actix_web::test]
async fn the_same_key_with_another_body_is_idempotency_conflict() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[JPY]).await;
    let key = unique("key");
    let first = post(&db, FRONT, &body("JPY", "BUY", "1000", None), Some(&key)).await;
    assert_eq!(first.status, StatusCode::OK, "body {}", first.text);
    let other = post(&db, FRONT, &body("JPY", "BUY", "2000", None), Some(&key)).await;
    assert_eq!(
        (other.status, other.code()),
        (StatusCode::CONFLICT, "idempotencyConflict"),
        "body {}",
        other.text
    );
    assert_eq!(db.quote_rows().await, (1, 1), "still one quote");
    db.finish().await;
}

#[actix_web::test]
async fn who_may_ask_is_todays_save_transaction_row_and_a_refusal_stores_nothing() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[JPY]).await;
    let mut callers = vec![String::new(), "owner".to_owned()];
    for role in STAFF_ROLES {
        callers.push(format!("staff:{role}:MAIN"));
        callers.push(format!("owner+staff:{role}:MAIN"));
    }
    let save = Action::new("saveTransaction");
    let mut written = 0;
    for who in &callers {
        // OR-4b (#767, design §4.2 C2): the route table's rule; ExchangeTransact is TELLER's, so the
        // owner's user is refused
        let allowed = blueprint_rule(
            actor_on_rpc(who).as_ref(),
            EntryPath::Rpc,
            save.as_str(),
            None,
        )
        .is_ok();
        let a = ask(&db, who, &body("JPY", "BUY", "1000", None)).await;
        if allowed {
            assert_eq!(a.status, StatusCode::OK, "{who:?} may ask: body {}", a.text);
            written += 1;
            let id = a.body["id"].as_str().expect("id");
            let actor = actor_on_rpc(who).expect("a signed-in caller");
            assert_eq!(
                db.text(&format!("SELECT created_by FROM quotes WHERE id = '{id}'"))
                    .await
                    .as_deref(),
                Some(actor.id()),
                "{who:?}: created_by is the caller"
            );
        } else {
            assert_ne!(a.status, StatusCode::OK, "{who:?} may not ask: {}", a.text);
            assert!(
                POLICY_CODES.contains(&a.code()),
                "{who:?}: a policy refusal, got {}",
                a.text
            );
        }
        assert_eq!(
            db.quote_rows().await,
            (written, written),
            "{who:?}: only allowed callers store a quote"
        );
    }
    // C2: FRONT (TELLER) alone is allowed
    assert_eq!(written, 1, "FRONT is allowed, got {written}");
    db.finish().await;
}

/// The branch of the stored quote `id` and the published line it was priced on.
async fn stored_branch_and_line(db: &TestDb, id: &str) -> (String, String) {
    sqlx::query_as(
        "SELECT q.branch_id, l.published_rate_line_id::text \
         FROM quotes q JOIN quote_lines l ON l.quote_id = q.id WHERE q.id = $1::uuid",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .expect("read quotes")
    .unwrap_or_else(|| panic!("the answered id {id} is a row of quotes"))
}

/// XC-B01 with JPY at 23.0 / 24.5 per 100, beside MAIN's 23.5 / 24.0.
const B01_JPY: Line = Line::manual("JPY", "100", "23.0", "24.5");

#[actix_web::test]
async fn a_staff_caller_naming_another_branch_is_out_of_scope_and_stores_nothing() {
    let db = TestDb::start().await;
    db.branch("XC-B01").await;
    let main = db.publish("MAIN", &[JPY]).await;
    db.publish("XC-B01", &[B01_JPY]).await;
    let a = ask(&db, FRONT, &on("XC-B01", body("JPY", "BUY", "1000", None))).await;
    refused(
        &db,
        &a,
        "outOfScope",
        StatusCode::FORBIDDEN,
        "MAIN's FRONT names XC-B01",
    )
    .await;
    // the same FRONT naming its own branch is answered on it
    let a = ask(&db, FRONT, &on("MAIN", body("JPY", "BUY", "1000", None))).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    let id = a.body["id"].as_str().expect("id");
    assert_eq!(
        stored_branch_and_line(&db, id).await,
        ("MAIN".to_owned(), main.line("JPY").to_owned()),
        "MAIN's quote on MAIN's line"
    );
    db.finish().await;
}

#[actix_web::test]
async fn the_owner_naming_a_branch_is_quoted_on_that_branchs_line() {
    let db = TestDb::start().await;
    db.branch("XC-B01").await;
    db.publish("MAIN", &[JPY]).await;
    let b01 = db.publish("XC-B01", &[B01_JPY]).await;
    // OR-4b (#767, design §4.2 C2): ExchangeTransact is TELLER's, so a TELLER (v2's FRONT) of
    // XC-B01 asks where v2's owner did, naming XC-B01
    let a = ask(
        &db,
        "staff:FRONT:XC-B01",
        &on("XC-B01", body("JPY", "BUY", "1000", None)),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(
        (
            &a.body["lines"][0]["thbAmount"],
            &a.body["lines"][0]["publishedRateLineId"],
            &a.body["publication"]["id"],
        ),
        (&json!("230.00"), &json!(b01.line("JPY")), &json!(b01.id)),
        "XC-B01's JPY at 23.0 per 100"
    );
    let id = a.body["id"].as_str().expect("id");
    assert_eq!(
        stored_branch_and_line(&db, id).await,
        ("XC-B01".to_owned(), b01.line("JPY").to_owned()),
        "quotes.branch_id is the seam's branch"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_branch_id_in_the_body_is_not_the_quotes_branch() {
    let db = TestDb::start().await;
    db.branch("XC-B01").await;
    let main = db.publish("MAIN", &[JPY]).await;
    db.publish("XC-B01", &[B01_JPY]).await;
    // MAIN's FRONT names no `branch`: the seam runs on MAIN whatever `branchId` says. OR-4b
    // (#767, C2): FRONT (TELLER) asks where v2's owner did
    let mut b = body("JPY", "BUY", "1000", None);
    b["branchId"] = json!("XC-B01");
    let a = ask(&db, FRONT, &b).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(
        a.body["lines"][0]["thbAmount"],
        json!("235.00"),
        "MAIN's rate"
    );
    let id = a.body["id"].as_str().expect("id");
    assert_eq!(
        stored_branch_and_line(&db, id).await,
        ("MAIN".to_owned(), main.line("JPY").to_owned()),
        "stored on MAIN"
    );
    db.finish().await;
}

// ------------------------------------------------------------ refusals

#[actix_web::test]
async fn a_body_the_quote_rules_refuse_is_validation_failed() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[JPY, USD]).await;

    let mut two = body("JPY", "BUY", "1000", None);
    two["lines"] = json!([
        { "currency": "JPY", "shopSide": "BUY", "quantity": "1000" },
        { "currency": "USD", "shopSide": "BUY", "quantity": "10" },
    ]);
    let cases: Vec<(&str, Value)> = vec![
        ("two lines", two),
        ("no line", {
            let mut b = body("JPY", "BUY", "1000", None);
            b["lines"] = json!([]);
            b
        }),
        (
            "JPY 1000.5 does not fit the minor unit",
            body("JPY", "BUY", "1000.5", None),
        ),
        (
            "USD 10.001 does not fit the minor unit",
            body("USD", "BUY", "10.001", None),
        ),
        ("quantity 0", body("USD", "BUY", "0", None)),
        (
            "BUY fee 32.11 above thb 32.10",
            body("USD", "BUY", "1", Some("32.11")),
        ),
        (
            "SELL fee 32.46 above thb 32.45",
            body("USD", "SELL", "1", Some("32.46")),
        ),
    ];
    for (what, b) in cases {
        let a = ask(&db, FRONT, &b).await;
        refused(&db, &a, "validationFailed", StatusCode::BAD_REQUEST, what).await;
    }
    db.finish().await;
}

#[actix_web::test]
async fn no_line_or_a_stale_line_under_stop_is_rate_unavailable() {
    let db = TestDb::start().await;
    let stop = db.policy("MAIN", Some("STOP"), None).await;
    db.publish(
        "MAIN",
        &[
            JPY,
            Line {
                stale_under: Some(stop),
                ..USD
            },
        ],
    )
    .await;
    let a = ask(&db, FRONT, &body("EUR", "BUY", "10", None)).await;
    refused(
        &db,
        &a,
        "rateUnavailable",
        StatusCode::CONFLICT,
        "EUR is not published",
    )
    .await;
    let a = ask(&db, FRONT, &body("USD", "BUY", "10", None)).await;
    refused(
        &db,
        &a,
        "rateUnavailable",
        StatusCode::CONFLICT,
        "USD is stale under STOP",
    )
    .await;
    // the fresh line is answered
    let a = ask(&db, FRONT, &body("JPY", "BUY", "1000", None)).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    db.finish().await;
}

// ------------------------------------------------------------ the TypeScript

#[test]
fn quote_dto_exports_to_quote_ts_with_every_field_and_money_as_strings() {
    let dir: PathBuf = std::env::temp_dir().join(unique("xc-quote-ts"));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let cfg = ts_rs::Config::new().with_out_dir(&dir);
    QuoteDto::export_all(&cfg).expect("export QuoteDto");
    let ts = std::fs::read_to_string(dir.join("quote.ts")).unwrap_or_else(|e| {
        panic!(
            "QuoteDto exports to quote.ts (ticket Part 2 file scope): {e}; wrote {:?}",
            std::fs::read_dir(&dir).map(|d| d
                .filter_map(|e| e.ok())
                .map(|e| e.file_name())
                .collect::<Vec<_>>())
        )
    });
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ts.contains("QuoteDto"), "quote.ts declares QuoteDto: {ts}");
    for key in [
        "id",
        "type",
        "channel",
        "expiresAt",
        "publication",
        "lines",
        "benefits",
        "stale",
        "currency",
        "shopSide",
        "publishedRateLineId",
    ] {
        assert!(ts.contains(&format!("{key}:")), "quote.ts has {key}: {ts}");
    }
    for key in [
        "quantity",
        "unitSize",
        "baseRate",
        "finalRate",
        "thbAmount",
        "feeThb",
        "settleThb",
    ] {
        let decl = ts
            .split(&format!("{key}:"))
            .nth(1)
            .unwrap_or_else(|| panic!("quote.ts has {key}: {ts}"));
        let ty = decl
            .trim_start()
            .split([',', ';', '}', ' '])
            .next()
            .unwrap_or("");
        assert!(
            ["string", "Money", "Rate"].contains(&ty),
            "{key} is a decimal string on the wire, not {ty:?}: {ts}"
        );
    }
}
