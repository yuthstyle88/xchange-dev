//! Ticket #2024 — HTTP route tests for `POST /api/reversals/{id}/post` (`reversalPost`),
//! pinning the branch ownership check at `api/src/presentation/http/v3/commands/reversal_post.rs:116`
//! (`Some(owner) if owner != branch.as_str() => Err(out_of_scope())`), unknown reversal id (`404 notFound`),
//! malformed id (`400 validationFailed`), and the `200` control case on `MAIN`.

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
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::reversal::reversal::ReversalId;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionId};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_referral::application::feed_writer::{
    CompletedExchange as FeedExchange, ExchangeFeedWriter,
};
use xc_referral::domain::customer_keys::CustomerKeys;
use xchange::application::workflows::exchange::complete::{CompleteExchange, ExchangeEffects};
use xchange::application::workflows::exchange::reversal_approval;
use xchange::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ME: &str = "teller";

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn satang(thb: Decimal) -> i64 {
    use rust_decimal::prelude::ToPrimitive;
    (thb * Decimal::from(100)).to_i64().unwrap()
}

fn finance() -> access::Actor {
    access::Actor::new(
        "finance-MAIN",
        "00000000-0000-0000-0000-000000000001",
        vec![access::ScopedRole::new(
            access::Role::Finance,
            Some(BranchId::new("MAIN")),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

/// Who is signed in, from `x-test-as`: `teller` (FRONT of MAIN, id `TELLER`), `staff:<ROLE>:<branch>`, or absent.
struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    let staff = match spec {
        "" => None,
        "teller" => Actor::staff(TELLER, Role::Front, BranchId::new("MAIN")),
        other => {
            let mut parts = other.splitn(3, ':');
            assert_eq!(parts.next(), Some("staff"), "staff spec {other:?}");
            let role = Role::parse(parts.next().expect("a role")).expect("a known role");
            let branch = parts.next().expect("a branch");
            Actor::staff(
                format!("employee-{}", role.as_str()),
                role,
                BranchId::new(branch),
            )
        }
    };
    Sessions { owner: None, staff }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
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

    fn field(&self) -> Option<&str> {
        self.body["error"]["field"].as_str()
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
        Shop {
            c,
            approvals,
            requests,
        }
    }

    async fn finish(self) {
        self.c.finish().await;
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.c.desk.db.pool
    }

    /// A LIVE exchange of USD `qty` completed at `now` + 5 s through TX-5 with the ledger's port, and its Broker feed
    /// row, as the completion's feed step writes it (`broker_feed_save_step.rs`).
    async fn completed(
        &self,
        side: Side,
        qty: &str,
        method: PaymentMethod,
        now: DateTime<Utc>,
    ) -> ExchangeTransaction {
        let q = self.c.desk.quote("MAIN", side, qty, None, now).await;
        let t = self.c.live_of(&q, method, true, now).await;
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
        let settle = self.settle(t.id).await;
        ExchangeFeedWriter::record_completed(
            &mut tx,
            &FeedExchange {
                transaction_id: t.id.0,
                organization_id: t.organization_id.to_string(),
                branch_id: "MAIN".to_owned(),
                keys: CustomerKeys {
                    customer_sub: None,
                    phone: None,
                },
                value_satang: satang(settle),
                completed_at: later(now, 5),
            },
        )
        .await
        .expect("the feed row of the completion");
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
                    key: next("rv3-request"),
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
                &next("rv3-decide"),
            )
            .await
            .unwrap_or_else(|e| panic!("FINANCE approves: {}", e.code()));
        tx.commit().await.unwrap();
        r.id
    }

    async fn dec(&self, sql: &str) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(self.pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .unwrap_or_default()
    }

    async fn int(&self, sql: &str) -> i64 {
        self.c.desk.db.count(sql).await
    }

    async fn text(&self, sql: &str) -> String {
        self.c.desk.db.text(sql).await.unwrap_or_default()
    }

    async fn settle(&self, t: TransactionId) -> Decimal {
        self.dec(&format!(
            "SELECT sum(settle_thb) FROM transaction_lines WHERE transaction_id = '{}'",
            t.0
        ))
        .await
    }

    async fn reversal(&self, id: ReversalId, column: &str) -> String {
        self.text(&format!(
            "SELECT {column}::text FROM reversals WHERE id = '{}'",
            id.0
        ))
        .await
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
}

/// `POST /api/reversals/{id}/post` as `who`, naming `branch` in the JSON body when given.
async fn post_reversal(
    s: &Shop,
    id: &str,
    who: &str,
    branch: Option<&str>,
    key: Option<&str>,
) -> Answer {
    let dir = s.c.scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        s.pool().clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let payload = match branch {
        Some(b) => json!({ "branch": b }),
        None => json!({}),
    };
    let mut req = atest::TestRequest::post()
        .uri(&format!("/api/reversals/{id}/post"))
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(payload.to_string());
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

/// Row 1: a teller of BR2 (the request names BR2) posts MAIN's APPROVED reversal, with MAIN's shift open ->
/// 403 `outOfScope`; the reversal stays APPROVED, no `receipts` row, no ledger posting.
#[actix_web::test]
async fn r1_teller_of_br2_posting_mains_approved_reversal_is_out_of_scope() {
    let s = Shop::start().await;
    let now = whole_second_now();
    let orig = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let rev = s.approved(&orig, now).await;

    let a = post_reversal(
        &s,
        &rev.0.to_string(),
        &format!("staff:FRONT:{BR2}"),
        Some(BR2),
        Some(&unique("key")),
    )
    .await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::FORBIDDEN, "outOfScope"),
        "body {}",
        a.text
    );
    assert_eq!(
        s.reversal(rev, "status").await,
        "APPROVED",
        "the reversal stays APPROVED"
    );
    assert_eq!(
        s.reversal_receipts(orig.id).await,
        0,
        "no reversal receipts row"
    );
    assert_eq!(s.reversal_postings(rev).await, 0, "no ledger posting");
    s.finish().await;
}

/// Row 2: unknown reversal uuid -> 404 `notFound`, field `id` (`reversal_post.rs:110-115`).
#[actix_web::test]
async fn r2_unknown_reversal_uuid_answers_not_found_on_id() {
    let s = Shop::start().await;
    let unknown = Uuid::new_v4().to_string();

    let a = post_reversal(&s, &unknown, ME, None, Some(&unique("key"))).await;
    assert_eq!(
        (a.status, a.code(), a.field()),
        (StatusCode::NOT_FOUND, "notFound", Some("id")),
        "body {}",
        a.text
    );
    s.finish().await;
}

/// Row 3: malformed id -> 400 `validationFailed`, field `id` (`reversal_post.rs:96-102`).
#[actix_web::test]
async fn r3_malformed_reversal_id_answers_validation_failed_on_id() {
    let s = Shop::start().await;

    let a = post_reversal(&s, "not-a-uuid", ME, None, Some(&unique("key"))).await;
    assert_eq!(
        (a.status, a.code(), a.field()),
        (StatusCode::BAD_REQUEST, "validationFailed", Some("id")),
        "body {}",
        a.text
    );
    s.finish().await;
}

/// Row 4: a teller of MAIN, APPROVED, open shift -> 200 (control).
#[actix_web::test]
async fn r4_teller_of_main_posts_approved_reversal_with_open_shift() {
    let s = Shop::start().await;
    let now = whole_second_now();
    let orig = s
        .completed(Side::Buy, "100", PaymentMethod::Cash, now)
        .await;
    let rev = s.approved(&orig, now).await;

    let a = post_reversal(&s, &rev.0.to_string(), ME, None, Some(&unique("key"))).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["reversal"]["id"], json!(rev.0.to_string()));
    assert_eq!(a.body["reversal"]["status"], "POSTED", "body {}", a.text);
    let doc = a.body["documentNumber"]
        .as_str()
        .unwrap_or_else(|| panic!("a documentNumber: {}", a.text));
    assert!(doc.ends_with("-R2"), "expected -R2 suffix, got {doc}");
    assert_eq!(s.reversal(rev, "status").await, "POSTED");
    assert_eq!(s.reversal_receipts(orig.id).await, 1);
    assert_eq!(s.reversal_postings(rev).await, 1);
    s.finish().await;
}
