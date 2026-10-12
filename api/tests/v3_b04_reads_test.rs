//! Ticket #1294 (TD-2) — the two B04 reads: `GET /api/v3/views/receipt_document?number=` answers a
//! stored receipt's HTML to a holder of `ReceiptReprint`, and `GET /api/v3/views/transaction_audit?id=`
//! answers a transaction's audit trail, with its reversals' events, to the readers of TX-10's
//! `transaction` view.
//! Written before the implementation by a rule-A test-writer (claude/89bd61d9, on the lane
//! controller's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/desk-transaction-detail.md` §4.2 (receipt document), §4.3 (audit),
//! D5 (the receipt is read through a view, not `/files/`), D7, D11, §5 row TD-2; Blueprint B04
//! (*"พิมพ์ซ้ำ"*, *"audit"*), B16 (*"A reprint prints the stored document again and writes nothing"*).
//!
//! What this test pins ([required] in ticket §7/§8 unless marked):
//! - the view `receipt_document`: filter `{ number }`, data `{ receipt: { number, revision, issuedAt,
//!   html } | null }`, exactly those four keys, `html` byte-equal to the stored file;
//! - the view `transaction_audit`: filter `{ id }`, data `{ events: [{ at, actorId, actorRole, action
//!   }] | null }`, exactly those four keys per event; the events whose `object_ref` is the
//!   transaction's id or one of its reversals' ids, oldest first (`at`, then `id`);
//! - **`null`, not an error, for anything not found** (unknown, malformed, another branch's, a
//!   receipt whose file is gone): a view's load error is a 500 (`v3/view.rs`), and TX-10 answers
//!   `null` the same way (`v3_transaction_view_test.rs`);
//! - the policy row `(EntryPath::Rpc, "receiptReprint") → Need::Permission(ReceiptReprint)`, on the
//!   path of TX-10's `transactionView` row (ticket §7 `[proposed]`: both B04 reads on one path; `Rpc`
//!   is also the fallback the ticket names). Holders: TELLER (v3 FRONT) and BRANCH_MANAGER (v3 MANAGER,
//!   BACKEND). Others are refused by the seam (`roleNotAllowedHere` on `Rpc`), with no `data`;
//! - `transaction_audit`'s readers are exactly the `transaction` view's: for the owner, nobody, and
//!   every v3 staff role, the two views answer the same status, and its `INHERITS` is the very pair
//!   TX-10's `views/transaction.rs` declares (read from the two source files, whitespace ignored:
//!   `ReceiptReprint` and `TransactionRead` have the same holders today, so no request can tell the
//!   rows apart). `receipt_document.rs` declares `INHERITS = (EntryPath::Rpc, "receiptReprint")`;
//! - a receipt's branch is its transaction's (ticket §2: `receipts` has no branch column), so another
//!   branch's number is `null` even when the file it names is readable at the caller's branch;
//! - a reprint writes nothing: every table's row count is unchanged and the `receipts` rows are
//!   byte-equal before and after two reads (an UPDATE is caught, not only an INSERT).
//!
//! Seeding (ticket §4 criterion 1): exchanges go through TX-2's store (`PgTransactionStore`:
//! `insert_draft_in`, `set_status_in`, `complete_in`), receipts are real files written by
//! `FileService::store_receipt_in` into the `APP_FILES__DIR` the app reads, the over-limit approval
//! and the reversal decision go through `ApprovalService::request`/`decide` (the real
//! `EXCHANGE_TRANSACTION` and `TRANSACTION_REVERSAL` subjects), the reversal through RV-2's
//! `RequestReversal`, and `transaction.created` through `PgAuditLog::append` with TX-11's shape
//! (`quote-transaction.md` §4.7). A revision-2 receipt is a second `receipts` row with its own number
//! and its own stored file (ticket §9); since RV-1 (V-4) a revision above 1 is `kind = 'REVERSAL'`.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Sessions come from the `x-test-as` header
//! (`owner`, `staff:<ROLE>:<branch>`), as in `v3_transaction_view_test.rs`.

mod common;
mod quote_common;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use quote_common::TestDb;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Permission;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xchange::application::workflows::exchange::approval as exchange_approval;
use xchange::application::workflows::exchange::reversal_approval;
use xchange::application::workflows::exchange::reversal_request::{
    RequestReversal, RequestReversalInput,
};
use xchange::application::workflows::save_step::SaveActor;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::files::file_service::FileService;
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const FRONT: &str = "staff:FRONT:MAIN";
const MANAGER: &str = "staff:MANAGER:MAIN";

/// Every v3 staff role (`xc_auth` `Role`, the owner aside).
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

// ── sessions ─────────────────────────────────────────────────────────────────

struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    if spec.is_empty() {
        return Sessions {
            owner: None,
            staff: None,
        };
    }
    if spec == "owner" {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        };
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Sessions {
        owner: None,
        staff: Actor::staff(
            format!("employee-{}", role.as_str()),
            role,
            BranchId::new(branch),
        ),
    }
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

// ── times ────────────────────────────────────────────────────────────────────

/// Completion: 2026-10-06 10:00 in Bangkok, MAIN's timezone.
fn completed_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 3, 0, 0).unwrap()
}

/// Inside RV-2's window for an exchange completed at `completed_at`.
fn inside_the_window() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 5, 0, 0).unwrap()
}

fn instant(v: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(v.as_str().unwrap_or_else(|| panic!("an instant, got {v}")))
        .unwrap_or_else(|e| panic!("{v} is RFC 3339: {e}"))
        .with_timezone(&Utc)
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap()
}

/// OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: the approval core takes
/// `access::Actor`, so this is the B1-mapped user of the v3 employee `role` on `branch` (FINANCE stays FINANCE,
/// so the stored `actor_role` `"FINANCE"` below is unchanged, ruling 5 on #1020).
fn staff(role: Role, branch: &str) -> xc_auth::domain::access::Actor {
    let v3 = Actor::staff(
        format!("employee-{}", role.as_str()),
        role,
        BranchId::new(branch),
    )
    .expect("a staff actor");
    xc_auth::domain::access::v3_roles::actor_from_v3(
        &v3,
        "00000000-0000-0000-0000-000000000001",
        chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
    )
}

fn teller() -> SaveActor {
    SaveActor {
        id: "employee-FRONT".to_owned(),
        username: "front".to_owned(),
        employee_id: Some("employee-FRONT".to_owned()),
        role: Role::Front,
    }
}

/// The HTML of a receipt, distinct per `tag`, with non-ASCII text so a lossy decode would show.
fn receipt_html(tag: &str) -> String {
    format!(
        "<!doctype html><html lang=\"th\"><head><meta charset=\"utf-8\"><title>ใบเสร็จ {tag}</title>\
         </head><body><h1>ใบรับซื้อเงินตราต่างประเทศ</h1><p>{tag} · USD 100 × 32.10 = ฿3,210.00</p>\
         <script>window.print()</script></body></html>\n"
    )
}

// ── the shop ─────────────────────────────────────────────────────────────────

/// One completed exchange: its id, its number (receipt revision 1) and its receipt's file key.
struct Completed {
    id: TransactionId,
    number: String,
    file_key: String,
    html: String,
}

struct Room {
    db: TestDb,
    dir: PathBuf,
    files: FileService,
    store: PgTransactionStore,
    approvals: Arc<ApprovalService>,
    reversals: RequestReversal,
    audit: PgAuditLog,
}

impl Room {
    async fn open() -> Self {
        let db = TestDb::start().await;
        db.branch("B2").await;
        for branch in ["MAIN", "B2"] {
            db.exec(&format!(
                "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
                 SELECT 'CU-{branch}', organization_id, 'CU-{branch}', 'Somchai', 'LIVE', now() \
                 FROM branches WHERE id = '{branch}'"
            ))
            .await;
            db.exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('PASSPORT-{branch}', '{branch}', 'owner', 'image/jpeg', 'sha-p-{branch}', now())"
            ))
            .await;
            // a teller user owns the shift (CA-1's `shifts.teller_id`)
            db.exec(&format!(
                "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                     SELECT organization_id, 'Teller {branch}', 'ACTIVE', 'test' FROM branches \
                     WHERE id = '{branch}' RETURNING id) \
                 INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, \
                     teller_id) \
                 SELECT 'SH-{branch}', '{branch}', 'OPEN', now(), 'owner', 'open-{branch}', u.id FROM u"
            ))
            .await;
        }
        let dir = std::env::temp_dir().join(quote_common::unique("xc-td2-files"));
        std::fs::create_dir_all(&dir).expect("create the files directory");
        let mut registry = ApprovalRegistry::new();
        exchange_approval::register(&mut registry, db.pool.clone());
        reversal_approval::register(&mut registry, db.pool.clone());
        let approvals = Arc::new(ApprovalService::new(
            registry,
            Arc::new(PgApprovalRepository::new(db.pool.clone())),
            Arc::new(PgAuditLog::new(db.pool.clone())),
        ));
        Self {
            files: FileService::new(db.pool.clone(), dir.clone()),
            store: PgTransactionStore::new(db.pool.clone()),
            reversals: RequestReversal::new(db.pool.clone(), approvals.clone()),
            audit: PgAuditLog::new(db.pool.clone()),
            approvals,
            dir,
            db,
        }
    }

    async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        self.db.finish().await;
    }

    /// A LIVE BUY of 100 USD at 32.1 at `branch`, in its shift, as a DRAFT (TX-2's shape).
    fn draft(&self, branch: &str) -> ExchangeTransaction {
        let id = TransactionId(Uuid::new_v4());
        ExchangeTransaction {
            id,
            organization_id: Uuid::parse_str(quote_common::ORG_1).unwrap(),
            branch_id: BranchId::new(branch.to_owned()),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: Some(format!("SH-{branch}")),
            submission_id: format!("sub-{}", id.0),
            payload_hash: "hash".to_owned(),
            customer_id: format!("CU-{branch}"),
            customer_name: "Somchai".to_owned(),
            nationality: "THA".to_owned(),
            staff_id: "EMP-001".to_owned(),
            booking_no: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
            recorded_at: completed_at(),
            recorded_by: "owner".to_owned(),
            completed_at: None,
            cancelled_at: None,
            cancel_reason: None,
            approval_request_id: None,
            passport_file_key: format!("PASSPORT-{branch}"),
            kiosk_request_id: None,
            staff_draft_id: None,
            count_confirmed: false,
            lines: vec![TransactionLine {
                id: Uuid::new_v4(),
                quote_line_id: None,
                shop_side: Side::Buy,
                currency: CurrencyCode::parse("USD").unwrap(),
                quantity: ForeignAmount::from_decimal(Decimal::from(100)),
                unit_size: UnitSize::parse("1").unwrap(),
                rate_snapshot: Rate::parse("32.1").unwrap(),
                reference_rate: None,
                thb_amount: money("3210.00"),
                fee_thb: money("10.00"),
                settle_thb: money("3200.00"),
                cost_basis_thb: None,
                realized_margin_thb: None,
                line_no: 1,
            }],
            payment: TransactionPayment {
                id: Uuid::new_v4(),
                payment_no: 1,
                method: PaymentMethod::Cash,
                reference: String::new(),
                account: "Cash drawer 1".to_owned(),
                slip_file_key: None,
                amount_thb: money("3200.00"),
                bank_fee_thb: money("0.00"),
                paid_at: completed_at(),
            },
            receipts: vec![],
        }
    }

    async fn insert(&self, t: &ExchangeTransaction) {
        let mut tx = self.db.pool.begin().await.unwrap();
        self.store
            .insert_draft_in(&mut tx, t)
            .await
            .expect("insert the draft");
        tx.commit().await.unwrap();
    }

    async fn move_to(
        &self,
        id: TransactionId,
        from: TransactionStatus,
        to: TransactionStatus,
        request: Option<Uuid>,
    ) {
        let mut tx = self.db.pool.begin().await.unwrap();
        self.store
            .set_status_in(
                &mut tx,
                id,
                StatusMove {
                    from,
                    to,
                    reason: None,
                    approval_request_id: request,
                },
                completed_at(),
            )
            .await
            .expect("move the status");
        tx.commit().await.unwrap();
    }

    /// Stores `html` as a receipt file of `branch` (`FileService::store_receipt_in`); answers its key.
    async fn store_receipt(&self, branch: &str, html: &str) -> String {
        let mut tx = self.db.pool.begin().await.unwrap();
        let key = self
            .files
            .store_receipt_in(
                &mut tx,
                &BranchId::new(branch),
                "owner",
                html,
                completed_at(),
            )
            .await
            .expect("store the receipt file");
        tx.commit().await.unwrap();
        key
    }

    /// CONFIRMED ⇒ COMPLETED with a stored receipt: TX-2's `complete_in` numbers it and writes
    /// receipt revision 1.
    async fn complete(&self, branch: &str, id: TransactionId, tag: &str) -> Completed {
        let html = receipt_html(tag);
        let file_key = self.store_receipt(branch, &html).await;
        let mut tx = self.db.pool.begin().await.unwrap();
        let (number, _receipt) = self
            .store
            .complete_in(&mut tx, id, &file_key, completed_at())
            .await
            .expect("complete the exchange");
        tx.commit().await.unwrap();
        Completed {
            id,
            number,
            file_key,
            html,
        }
    }

    /// A completed exchange at `branch` (DRAFT ⇒ CONFIRMED ⇒ COMPLETED), with receipt revision 1.
    async fn completed(&self, branch: &str, tag: &str) -> Completed {
        let t = self.draft(branch);
        self.insert(&t).await;
        self.move_to(
            t.id,
            TransactionStatus::Draft,
            TransactionStatus::Confirmed,
            None,
        )
        .await;
        self.complete(branch, t.id, tag).await
    }

    /// Revision 2: a second `receipts` row with its own number and its own stored file. Since RV-1
    /// (`20261112110000_reversals.sql`, V-4) a revision above 1 is `kind = 'REVERSAL'`, a reversal's
    /// document. Answers (number, html).
    async fn revision_2(&self, branch: &str, c: &Completed) -> (String, String) {
        let html = receipt_html(&format!("{}-revision-2", c.number));
        let key = self.store_receipt(branch, &html).await;
        let number = format!("{}-R2", c.number);
        self.db
            .exec(&format!(
                "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, kind, \
                    issued_at, document_ref, created_at, updated_at) \
                 SELECT gen_random_uuid(), organization_id, id, '{number}', 2, 'REVERSAL', \
                    '2026-10-06T04:00:00Z', '{key}', now(), now() FROM transactions WHERE id = '{}'",
                c.id.0
            ))
            .await;
        (number, html)
    }

    /// TX-11's `transaction.created` row (`quote-transaction.md` §4.7), through `PgAuditLog::append`.
    async fn created_event(&self, branch: &str, id: TransactionId) {
        let mut tx = self.db.pool.begin().await.unwrap();
        self.audit
            .append(
                &mut tx,
                &AuditEvent {
                    actor_id: "employee-FRONT".to_owned(),
                    actor_role: "FRONT".to_owned(),
                    scope: "branch".to_owned(),
                    action: "transaction.created".to_owned(),
                    object_ref: id.0.to_string(),
                    branch_id: Some(branch.to_owned()),
                    before: None,
                    after: Some(json!({ "status": "DRAFT", "quoteId": null })),
                },
            )
            .await
            .expect("append transaction.created");
        tx.commit().await.unwrap();
    }

    /// An over-limit exchange: `transaction.created`, then an `EXCHANGE_TRANSACTION` request decided
    /// APPROVED by the branch's MANAGER through `ApprovalService::decide` (`approval.approved`).
    async fn approved_over_the_limit(&self, branch: &str) -> TransactionId {
        let t = self.draft(branch);
        self.insert(&t).await;
        self.created_event(branch, t.id).await;
        let mut tx = self.db.pool.begin().await.unwrap();
        let request = self
            .approvals
            .request(
                &mut tx,
                exchange_approval::EXCHANGE_TRANSACTION,
                &t.id.0.to_string(),
                &json!({ "reason": "over the limit" }),
                &staff(Role::Front, branch),
                &format!("over-{}", t.id.0),
            )
            .await
            .unwrap_or_else(|e| panic!("request the approval: {}", e.code()));
        tx.commit().await.unwrap();
        let request_id = Uuid::parse_str(request.id.as_str()).expect("a uuid request id");
        self.move_to(
            t.id,
            TransactionStatus::Draft,
            TransactionStatus::PendingApproval,
            Some(request_id),
        )
        .await;
        self.decide(request_id, &staff(Role::Manager, branch)).await;
        t.id
    }

    async fn decide(&self, request: Uuid, who: &xc_auth::domain::access::Actor) {
        let mut tx = self.db.pool.begin().await.unwrap();
        self.approvals
            .decide(
                &mut tx,
                &RequestId::new(request.to_string()),
                who,
                Outcome::Approved,
                "checked",
                &format!("decide-{request}"),
            )
            .await
            .unwrap_or_else(|e| panic!("decide {request}: {}", e.code()));
        tx.commit().await.unwrap();
    }

    /// A reversal of `c` requested by the teller (RV-2) and APPROVED by the branch's FINANCE.
    /// Answers the reversal's id.
    async fn reversed(&self, branch: &str, c: &Completed) -> Uuid {
        let mut tx = self.db.pool.begin().await.unwrap();
        let reversal = self
            .reversals
            .run_in(
                &mut tx,
                RequestReversalInput {
                    branch: BranchId::new(branch),
                    original: c.id,
                    reason: "customer returned".to_owned(),
                    key: format!("reverse-{}", c.id.0),
                },
                &teller(),
                inside_the_window(),
            )
            .await
            .unwrap_or_else(|e| panic!("request the reversal: {e:?}"));
        tx.commit().await.unwrap();
        self.decide(reversal.approval_request_id, &staff(Role::Finance, branch))
            .await;
        reversal.id.0
    }

    /// (at, actor_id, actor_role, action) of the `audit_events` rows for these object refs, oldest
    /// first, read straight from the table.
    async fn events_of(&self, refs: &[String]) -> Vec<(DateTime<Utc>, String, String, String)> {
        sqlx::query_as(
            "SELECT at, actor_id, actor_role, action FROM audit_events \
             WHERE object_ref = ANY($1) ORDER BY at, id",
        )
        .bind(refs)
        .fetch_all(&self.db.pool)
        .await
        .expect("read audit_events")
    }

    /// Every base table of this test's schema with its row count.
    async fn row_counts(&self) -> Vec<(String, i64)> {
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT table_name::text FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE' ORDER BY 1",
        )
        .fetch_all(&self.db.pool)
        .await
        .expect("list the tables");
        let mut out = Vec::with_capacity(tables.len());
        for t in tables {
            let n = self
                .db
                .count(&format!("SELECT count(*) FROM \"{t}\""))
                .await;
            out.push((t, n));
        }
        out
    }

    async fn receipts_rows(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT row_to_json(r)::text FROM receipts r ORDER BY id")
            .fetch_all(&self.db.pool)
            .await
            .expect("read receipts")
    }

    // ── the app ──────────────────────────────────────────────────────────────

    async fn get(&self, who: &str, path: &str) -> (StatusCode, Value) {
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_FILES__DIR",
                self.dir.to_str().expect("a UTF-8 path"),
            )])),
            self.db.pool.clone(),
        );
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let mut req = atest::TestRequest::get()
            .uri(path)
            .insert_header((header::HOST, "xc.test"));
        if !who.is_empty() {
            req = req.insert_header((AS, who));
        }
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, body)
    }

    /// `data.receipt` of `receipt_document?number=<number>` as `who`; panics unless 200.
    async fn receipt(&self, who: &str, number: &str) -> Value {
        let (status, body) = self
            .get(
                who,
                &format!("/api/v3/views/receipt_document?number={number}"),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{who} receipt {number}: {body}");
        body.pointer("/data/receipt")
            .unwrap_or_else(|| panic!("{who} receipt {number}: no data.receipt in {body}"))
            .clone()
    }

    /// `data.events` of `transaction_audit?id=<id>` as `who`; panics unless 200.
    async fn events(&self, who: &str, id: &str) -> Value {
        let (status, body) = self
            .get(who, &format!("/api/v3/views/transaction_audit?id={id}"))
            .await;
        assert_eq!(status, StatusCode::OK, "{who} audit {id}: {body}");
        body.pointer("/data/events")
            .unwrap_or_else(|| panic!("{who} audit {id}: no data.events in {body}"))
            .clone()
    }
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v
        .as_object()
        .unwrap_or_else(|| panic!("an object, got {v}"))
        .keys()
        .cloned()
        .collect();
    k.sort();
    k
}

/// The view's events as (at, actorId, actorRole, action), checking each has exactly those keys.
fn answered(events: &Value) -> Vec<(DateTime<Utc>, String, String, String)> {
    events
        .as_array()
        .unwrap_or_else(|| panic!("events is an array, got {events}"))
        .iter()
        .map(|e| {
            assert_eq!(
                keys(e),
                ["action", "actorId", "actorRole", "at"],
                "exactly the four fields, no before/after, scope or branch: {e}"
            );
            let s = |k: &str| {
                e[k].as_str()
                    .unwrap_or_else(|| panic!("{k} is a string in {e}"))
                    .to_owned()
            };
            (instant(&e["at"]), s("actorId"), s("actorRole"), s("action"))
        })
        .collect()
}

// ── receipt_document (§4.2) ──────────────────────────────────────────────────

#[tokio::test]
async fn a_receipt_of_the_callers_branch_is_its_stored_html_byte_for_byte() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "first").await;
    let r = room.receipt(FRONT, &c.number).await;

    assert_eq!(
        keys(&r),
        ["html", "issuedAt", "number", "revision"],
        "exactly the four fields: {r}"
    );
    assert_eq!(r["number"], json!(c.number));
    assert_eq!(r["revision"], json!(1));
    assert_eq!(instant(&r["issuedAt"]), completed_at());
    let html = r["html"].as_str().expect("html is a string");
    let stored = std::fs::read(room.dir.join(&c.file_key)).expect("the stored receipt file");
    assert_eq!(
        html.as_bytes(),
        stored.as_slice(),
        "D5: the stored file's bytes, unchanged"
    );
    assert_eq!(html, c.html);
    room.finish().await;
}

#[tokio::test]
async fn revision_2_answers_its_own_stored_html() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "first").await;
    let (number, html) = room.revision_2("MAIN", &c).await;
    let r = room.receipt(MANAGER, &number).await;
    assert_eq!(r["number"], json!(number));
    assert_eq!(r["revision"], json!(2));
    assert_eq!(r["html"], json!(html), "revision 2's own document");
    assert_ne!(r["html"], json!(c.html), "not revision 1's");
    let first = room.receipt(MANAGER, &c.number).await;
    assert_eq!(
        (first["revision"].clone(), first["html"].clone()),
        (json!(1), json!(c.html)),
        "revision 1 is still its own"
    );
    room.finish().await;
}

#[tokio::test]
async fn an_unknown_malformed_or_other_branchs_number_answers_null() {
    let room = Room::open().await;
    let main = room.completed("MAIN", "main").await;
    let b2 = room.completed("B2", "b2").await;

    assert_eq!(
        room.receipt(FRONT, "XC-20991231-999999").await,
        Value::Null,
        "unknown"
    );
    assert_eq!(
        room.receipt(FRONT, "not%20a%20number").await,
        Value::Null,
        "malformed"
    );
    assert_eq!(
        room.receipt(FRONT, &b2.number).await,
        Value::Null,
        "MAIN's teller does not read B2's receipt"
    );
    assert_eq!(
        room.receipt("staff:FRONT:B2", &main.number).await,
        Value::Null,
        "B2's teller does not read MAIN's receipt"
    );
    assert_eq!(
        room.receipt("staff:FRONT:B2", &b2.number).await["number"],
        json!(b2.number),
        "B2's teller reads B2's: the nulls above are the branch scope, not a missing row"
    );
    room.finish().await;
}

#[tokio::test]
async fn a_receipt_whose_file_is_gone_from_disk_answers_null() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "gone").await;
    assert_eq!(
        room.receipt(FRONT, &c.number).await["number"],
        json!(c.number),
        "readable while its file is there"
    );
    std::fs::remove_file(room.dir.join(&c.file_key)).expect("remove the stored file");
    assert_eq!(
        room.receipt(FRONT, &c.number).await,
        Value::Null,
        "D11: document not found, not an error"
    );
    room.finish().await;
}

#[tokio::test]
async fn teller_and_branch_manager_reprint_it() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "readers").await;
    for who in [
        "staff:FRONT:MAIN",
        "staff:MANAGER:MAIN",
        "staff:BACKEND:MAIN",
    ] {
        let r = room.receipt(who, &c.number).await;
        assert_eq!(r["number"], json!(c.number), "{who}");
        assert_eq!(r["html"], json!(c.html), "{who}");
    }
    room.finish().await;
}

/// A caller without `ReceiptReprint` is refused by the seam's policy check on `Rpc`
/// (`roleNotAllowedHere`, a client error), with no `data`. The owner (EXECUTIVE + ADMIN), RATES
/// (RATE_MANAGER), FINANCE and MONITOR (VERIFICATION_REVIEWER) hold no `ReceiptReprint`.
#[tokio::test]
async fn a_caller_without_receipt_reprint_is_refused_with_no_body() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "refused").await;
    for who in [
        "owner",
        "staff:RATES:MAIN",
        "staff:FINANCE:MAIN",
        "staff:MONITOR:MAIN",
    ] {
        let (status, body) = room
            .get(
                who,
                &format!("/api/v3/views/receipt_document?number={}", c.number),
            )
            .await;
        assert!(
            status.is_client_error(),
            "{who}: refused as a client error, got {status}: {body}"
        );
        assert_eq!(
            body["error"]["code"],
            json!("roleNotAllowedHere"),
            "{who}: the policy code main gives on Rpc: {body}"
        );
        assert!(body.get("data").is_none(), "{who}: no data: {body}");
    }
    room.finish().await;
}

#[tokio::test]
async fn nobody_signed_in_is_refused_sign_in_by_both_views() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "anon").await;
    for path in [
        format!("/api/v3/views/receipt_document?number={}", c.number),
        format!("/api/v3/views/transaction_audit?id={}", c.id.0),
    ] {
        let (status, body) = room.get("", &path).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: {body}");
        assert_eq!(body["error"]["code"], json!("signIn"), "{path}: {body}");
    }
    room.finish().await;
}

/// B16: a reprint writes nothing. Two reads leave every table's row count as it was, and the
/// `receipts` rows byte-equal (so an UPDATE is caught, not only an INSERT).
#[tokio::test]
async fn two_reprints_write_nothing() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "nothing-written").await;
    let counts = room.row_counts().await;
    let receipts = room.receipts_rows().await;
    assert!(
        counts.iter().any(|(t, n)| t == "audit_events" && *n >= 0),
        "audit_events is counted: {counts:?}"
    );
    for _ in 0..2 {
        let r = room.receipt(FRONT, &c.number).await;
        assert_eq!(
            r["number"],
            json!(c.number),
            "each read answers the receipt"
        );
    }
    assert_eq!(room.row_counts().await, counts, "no table's count moved");
    assert_eq!(
        room.receipts_rows().await,
        receipts,
        "the receipts rows are unchanged"
    );
    room.finish().await;
}

#[test]
fn the_receipt_reprint_row_needs_receipt_reprint() {
    assert_eq!(
        Policy::need(EntryPath::Rpc, &Action::new("receiptReprint")),
        Some(Need::Permission(Permission::ReceiptReprint)),
        "areas/desk_transaction.rs, on the path of TX-10's transactionView row"
    );
}

/// The receipt's branch is its transaction's, not its file's: a B2 exchange whose receipt names a
/// file stored at MAIN is still B2's, so MAIN's teller gets `null` for its number.
#[tokio::test]
async fn another_branchs_receipt_is_null_even_when_its_file_is_readable_here() {
    let room = Room::open().await;
    let main_file = room.store_receipt("MAIN", &receipt_html("main-file")).await;
    let t = room.draft("B2");
    room.insert(&t).await;
    room.move_to(
        t.id,
        TransactionStatus::Draft,
        TransactionStatus::Confirmed,
        None,
    )
    .await;
    let mut tx = room.db.pool.begin().await.unwrap();
    let (number, _) = room
        .store
        .complete_in(&mut tx, t.id, &main_file, completed_at())
        .await
        .expect("complete B2's exchange");
    tx.commit().await.unwrap();
    assert!(
        room.files
            .open_in_branch(&main_file, &BranchId::new("MAIN"))
            .await
            .expect("open")
            .is_some(),
        "the file is readable at MAIN"
    );
    assert_eq!(
        room.receipt(FRONT, &number).await,
        Value::Null,
        "B2's number, so not MAIN's, whatever branch its file is in"
    );
    room.finish().await;
}

/// `const INHERITS: … = (…);` of a view source file, with every whitespace removed.
fn inherits_in(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/presentation/http/v3/views")
        .join(file);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} exists: {e}", path.display()));
    let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let start = flat
        .find("constINHERITS:")
        .unwrap_or_else(|| panic!("{file} declares INHERITS"));
    let rest = &flat[start..];
    let end = rest.find(';').expect("INHERITS ends with ;");
    let decl = &rest[..end];
    decl[decl.find('=').expect("INHERITS = …") + 1..].to_owned()
}

#[test]
fn receipt_document_inherits_the_receipt_reprint_row() {
    assert_eq!(
        inherits_in("receipt_document.rs"),
        "(EntryPath::Rpc,\"receiptReprint\")",
        "ticket §7: INHERITS = (<path>, \"receiptReprint\"), on TX-10's path"
    );
}

#[test]
fn transaction_audit_inherits_exactly_the_transaction_views_row() {
    assert_eq!(
        inherits_in("transaction_audit.rs"),
        inherits_in("transaction.rs"),
        "ticket §7: the same (EntryPath, action) pair as TX-10's `transaction` view, no second row"
    );
}

// ── transaction_audit (§4.3) ─────────────────────────────────────────────────

/// An over-limit approval decided, with TX-11's `transaction.created`: two events, oldest first, both
/// on the transaction, each exactly `{at, actorId, actorRole, action}`.
#[tokio::test]
async fn an_approved_over_the_limit_exchange_answers_created_then_approved() {
    let room = Room::open().await;
    let id = room.approved_over_the_limit("MAIN").await;
    let got = answered(&room.events(MANAGER, &id.0.to_string()).await);
    let want = room.events_of(&[id.0.to_string()]).await;
    assert_eq!(
        got.iter().map(|e| e.3.as_str()).collect::<Vec<_>>(),
        ["transaction.created", "approval.approved"],
        "the two events, oldest first: {got:?}"
    );
    assert_eq!(
        got, want,
        "at, actorId, actorRole, action as stored, in (at, id) order"
    );
    assert_eq!(
        (got[0].1.as_str(), got[0].2.as_str()),
        ("employee-FRONT", "FRONT"),
        "who created it"
    );
    room.finish().await;
}

/// A reversal requested (RV-2) and decided by FINANCE: the reversal's decision event (`object_ref` =
/// the reversal id) is part of the original's trail.
#[tokio::test]
async fn a_reversals_decision_is_in_the_originals_trail() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "reversed").await;
    room.created_event("MAIN", c.id).await;
    let reversal = room.reversed("MAIN", &c).await;

    let reversal_events = room.events_of(&[reversal.to_string()]).await;
    assert!(
        reversal_events
            .iter()
            .any(|e| e.3 == "approval.approved" && e.2 == "FINANCE"),
        "FINANCE's decision is stored on the reversal: {reversal_events:?}"
    );
    let got = answered(&room.events(MANAGER, &c.id.0.to_string()).await);
    let want = room
        .events_of(&[c.id.0.to_string(), reversal.to_string()])
        .await;
    assert_eq!(
        got, want,
        "the transaction's and its reversal's events, oldest first"
    );
    assert_eq!(
        got.iter().map(|e| e.3.as_str()).collect::<Vec<_>>(),
        ["transaction.created", "approval.approved"],
        "{got:?}"
    );
    room.finish().await;
}

/// Another transaction's events, and its reversal's, are never in this one's trail.
#[tokio::test]
async fn another_transactions_events_and_its_reversals_are_never_included() {
    let room = Room::open().await;
    let mine = room.completed("MAIN", "mine").await;
    room.created_event("MAIN", mine.id).await;
    let theirs = room.completed("MAIN", "theirs").await;
    room.created_event("MAIN", theirs.id).await;
    let their_reversal = room.reversed("MAIN", &theirs).await;
    let other = room.approved_over_the_limit("MAIN").await;

    let got = answered(&room.events(FRONT, &mine.id.0.to_string()).await);
    let want = room.events_of(&[mine.id.0.to_string()]).await;
    assert_eq!(got, want, "only this transaction's own events");
    assert_eq!(
        got.len(),
        1,
        "its transaction.created only, not {their_reversal} nor {}'s: {got:?}",
        other.0
    );
    room.finish().await;
}

#[tokio::test]
async fn a_transaction_with_no_events_answers_an_empty_list() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "quiet").await;
    assert_eq!(
        room.events(FRONT, &c.id.0.to_string()).await,
        json!([]),
        "found, no events: [] (not null)"
    );
    room.finish().await;
}

#[tokio::test]
async fn an_unknown_malformed_or_other_branchs_id_answers_null() {
    let room = Room::open().await;
    let main = room.approved_over_the_limit("MAIN").await;
    let b2 = room.approved_over_the_limit("B2").await;

    assert_eq!(
        room.events(FRONT, &Uuid::new_v4().to_string()).await,
        Value::Null,
        "unknown"
    );
    assert_eq!(
        room.events(FRONT, "not-a-uuid").await,
        Value::Null,
        "malformed"
    );
    assert_eq!(
        room.events(FRONT, &b2.0.to_string()).await,
        Value::Null,
        "MAIN's teller does not read B2's trail"
    );
    assert_eq!(
        room.events("staff:FRONT:B2", &main.0.to_string()).await,
        Value::Null,
        "B2's teller does not read MAIN's trail"
    );
    assert_eq!(
        answered(&room.events("staff:FRONT:B2", &b2.0.to_string()).await).len(),
        2,
        "B2's teller reads B2's: the nulls above are the branch scope, not a missing row"
    );
    room.finish().await;
}

/// §4.3, last row: `transaction_audit`'s readers are exactly the `transaction` view's. For the
/// owner, nobody signed in, and every v3 staff role, the two views answer the same status.
#[tokio::test]
async fn its_readers_are_exactly_the_transaction_views() {
    let room = Room::open().await;
    let c = room.completed("MAIN", "readers").await;
    let mut callers = vec![String::new(), "owner".to_owned()];
    for role in STAFF_ROLES {
        callers.push(format!("staff:{role}:MAIN"));
    }
    let mut readers = 0;
    for who in &callers {
        let (view, view_body) = room
            .get(who, &format!("/api/v3/views/transaction?id={}", c.id.0))
            .await;
        let (audit, audit_body) = room
            .get(
                who,
                &format!("/api/v3/views/transaction_audit?id={}", c.id.0),
            )
            .await;
        assert_eq!(
            audit, view,
            "{who:?}: transaction_audit {audit} ({audit_body}) vs transaction {view} ({view_body})"
        );
        if audit == StatusCode::OK {
            readers += 1;
            assert!(
                audit_body.pointer("/data/events").is_some(),
                "{who:?}: {audit_body}"
            );
        } else {
            assert!(
                audit_body.get("data").is_none(),
                "{who:?}: a refusal carries no data: {audit_body}"
            );
        }
    }
    assert!(
        readers >= 2,
        "some callers do read it (TELLER, BRANCH_MANAGER)"
    );
    room.finish().await;
}
