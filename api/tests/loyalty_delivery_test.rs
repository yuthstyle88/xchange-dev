//! Ticket LY-2 (#1383) up-front tests — the two loyalty outbox rows are delivered to xchange's own Loyalty
//! (design `docs/design/blueprint/membership-points.md` §4.5 the wire contract, §7 edge cases, §9.3 row LY-2).
//! Written before the implementation by the test-writer claude/5d45b017, on the lane controller's order
//! (claude/ad05bf8d). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! **Black-box**, per the lane controller's rulings on #1383 (issuecomment-6116511363, -6116529436):
//! - configuration only through `XC_LOYALTY_BASE_URL`, `XC_LOYALTY_TENANT_ID`, `XC_LOYALTY_WEBHOOK_SECRET`
//!   and the optional `XC_LOYALTY_TIMEOUT_MS` (here 500), read by `Settings`;
//! - the handlers only through `outbox_handlers::registry(&state)` (two files, one per kind);
//! - delivery only through OB-1's real `OutboxWorker` and `PgOutboxStore`;
//! - Loyalty is a local fake on `127.0.0.1:0` that records each request and answers as scripted (no real
//!   network);
//! - the rows are LY-1's own (`LoyaltyEvents::completed_in` / `reversed_in`); the exchange, its line and
//!   the reversal are seeded with SQL in TX-2's and RV-1's schema; the customer's PHONE identity is
//!   seeded in CU-L1's storage form (the normalised E.164 sealed with AAD `customer_id ‖ "PHONE"`, its
//!   E.164 `lookup_hash`).
//!
//! The handler's constructor and how it reads the phone are the implementer's (**[proposed]**); nothing
//! here names them.
//!
//! Red by result on LY-1's implementation: no loyalty handler is registered, so the fake sees no request
//! and the rows stay `PENDING`. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod quote_common;

use std::collections::{BTreeMap, VecDeque};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Timelike, Utc};
use chrono_tz::Asia::Bangkok;
use quote_common::{ORG_1, TestDb};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;
use xc_kernel::Mode;
use xc_kernel::money::Side;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys};
use xc_loyalty::application::events::LoyaltyEvents;
use xc_loyalty::domain::customer_ref::LoyaltyCustomerRef;
use xc_loyalty::domain::events::{LOYALTY_COMPLETED, LOYALTY_REVERSED};
use xc_loyalty::domain::facts::{CompletedFacts, CompletedLine, ReversedFacts};
use xc_outbox::application::worker::{OutboxWorker, TickReport};
use xc_outbox::infrastructure::pg_store::PgOutboxStore;
use xchange::bootstrap::{AppState, outbox_handlers};

/// The PII key every test binary installs (`common::install_test_pii_keys`).
const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const TENANT: &str = "6c1f3a52-0d4e-4f7a-9b21-5e8c7d4a3b10";
const SECRET: &str = "whsec-LY2-7f3e91c0a5d24b68";
const PHONE: &str = "0812345678";
const PHONE_E164: &str = "+66812345678";
/// 20:00 UTC = 03:00 the next day in Bangkok: the business date is 2026-10-07, not the UTC date.
const COMPLETED_AT: &str = "2026-10-06T20:00:00Z";
const POSTED_AT: &str = "2026-10-06T21:00:00Z";

// ---------------------------------------------------------------- the fake Loyalty

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("the request has a {name} header: {:?}", self.headers))
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("the body is JSON ({e}): {}", self.body))
    }
}

#[derive(Clone)]
enum Answer {
    Status(u16),
    /// Accept the request and never answer.
    Hang,
}

#[derive(Clone)]
struct FakeLoyalty {
    url: String,
    answers: Arc<Mutex<VecDeque<Answer>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeLoyalty {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let answers: Arc<Mutex<VecDeque<Answer>>> = Arc::default();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let (a, s) = (answers.clone(), seen.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (a, s) = (a.clone(), s.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let head_end = loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                    let mut lines = head.split("\r\n");
                    let mut first = lines.next().unwrap_or("").split(' ');
                    let method = first.next().unwrap_or("").to_owned();
                    let path = first.next().unwrap_or("").to_owned();
                    let headers: BTreeMap<String, String> = lines
                        .filter_map(|l| l.split_once(':'))
                        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
                        .collect();
                    let len: usize = headers
                        .get("content-length")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    while buf.len() < head_end + len {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let body = String::from_utf8_lossy(&buf[head_end..]).into_owned();
                    s.lock().unwrap().push(Seen {
                        method,
                        path,
                        headers,
                        body,
                    });
                    let answer = a.lock().unwrap().pop_front().unwrap_or(Answer::Status(200));
                    match answer {
                        Answer::Hang => tokio::time::sleep(Duration::from_secs(60)).await,
                        Answer::Status(code) => {
                            let body = r#"{"outcome":"accrued"}"#;
                            let out = format!(
                                "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = sock.write_all(out.as_bytes()).await;
                            let _ = sock.shutdown().await;
                        }
                    }
                });
            }
        });
        Self { url, answers, seen }
    }

    fn answer(&self, a: Answer) {
        self.answers.lock().unwrap().push_back(a);
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------- the signature, by hand

/// HMAC-SHA256 (RFC 2104) over SHA-256, lowercase hex: what `pos_kernel::webhook_signature::sign_hex`
/// computes for `"<ts>.<body>"`.
fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let ipad: Vec<u8> = block.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = block.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::new()
        .chain_update(&ipad)
        .chain_update(message)
        .finalize();
    let outer = Sha256::new()
        .chain_update(&opad)
        .chain_update(inner)
        .finalize();
    outer.iter().map(|b| format!("{b:02x}")).collect()
}

/// The shared verifier's rule (`parse_signature` + `sign_hex`): `t=<ts>,v1=<hex>`, and `hex` is the
/// HMAC of `"<ts>.<raw body>"` under the webhook secret.
fn assert_signed(seen: &Seen) {
    let header = seen.header("x-pos108-signature");
    let mut ts = None;
    let mut v1 = None;
    for part in header.split(',') {
        match part.split_once('=') {
            Some(("t", v)) => ts = Some(v.trim()),
            Some(("v1", v)) => v1 = Some(v.trim()),
            _ => {}
        }
    }
    let (ts, v1) = (
        ts.unwrap_or_else(|| panic!("{header}: has t=")),
        v1.unwrap_or_else(|| panic!("{header}: has v1=")),
    );
    assert!(!ts.is_empty(), "{header}: t is not empty");
    let message = format!("{ts}.{}", seen.body);
    assert_eq!(
        v1,
        hmac_sha256_hex(SECRET.as_bytes(), message.as_bytes()),
        "v1 is HMAC-SHA256(secret, \"<ts>.<body>\") of the exact body sent"
    );
}

// ---------------------------------------------------------------- seeding

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .unwrap_or_else(|e| panic!("{s}: {e}"))
        .with_timezone(&Utc)
}

fn instant(v: &Value) -> DateTime<Utc> {
    at(v.as_str()
        .unwrap_or_else(|| panic!("an RFC 3339 string, got {v}")))
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal literal")
}

fn org() -> Uuid {
    Uuid::parse_str(ORG_1).expect("ORG_1 is a uuid")
}

fn keys() -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(TEST_KEYS).expect("the test PII key parses"))
}

#[derive(Clone, Copy)]
enum Phone {
    ActiveVerified,
    ActiveUnverified,
    RevokedVerified,
}

/// One seeded LIVE exchange of one customer.
struct Exchange {
    customer: String,
    facts: CompletedFacts,
}

struct World {
    db: TestDb,
    fake: FakeLoyalty,
    worker: OutboxWorker,
}

impl World {
    /// A migrated schema, MAIN with an open shift and a receipt file, Loyalty configured at the fake.
    async fn start() -> Self {
        let fake = FakeLoyalty::start().await;
        let url = fake.url.clone();
        Self::with_base_url(fake, &url).await
    }

    async fn with_base_url(fake: FakeLoyalty, base_url: &str) -> Self {
        let _ = xc_customer::install_pii_keys(keys());
        let db = TestDb::start().await;
        db.exec(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT id, 'ly2-teller', 'PENDING', 'test' FROM organizations WHERE code = 'XC'",
        )
        .await;
        db.exec(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('ly2-shift', 'MAIN', 'OPEN', now(), 'owner', 'ly2-open', \
               (SELECT id FROM users WHERE name = 'ly2-teller' AND created_by = 'test' ORDER BY created_at, id LIMIT 1))",
        )
        .await;
        db.exec(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('RECEIPT-LY2', 'MAIN', 'owner', 'text/html', 'sha-ly2', now())",
        )
        .await;
        let settings = common::settings(&[
            ("XC_LOYALTY_BASE_URL", base_url),
            ("XC_LOYALTY_TENANT_ID", TENANT),
            ("XC_LOYALTY_WEBHOOK_SECRET", SECRET),
            ("XC_LOYALTY_TIMEOUT_MS", "500"),
        ]);
        let state = AppState::new(Arc::new(settings), db.pool.clone());
        let registry =
            outbox_handlers::registry(&state).expect("the handlers register without a clash");
        let worker = OutboxWorker::new(
            Arc::new(PgOutboxStore::new(db.pool.clone())),
            Arc::new(registry),
        );
        Self { db, fake, worker }
    }

    async fn tick(&self) -> TickReport {
        tokio::time::timeout(Duration::from_secs(30), self.worker.tick())
            .await
            .expect("one tick ends: the handler's request has a timeout")
            .expect("the tick answers Ok")
    }

    /// A LIVE customer of MAIN with the given PHONE identities (CU-L1's storage form).
    async fn customer(&self, phones: &[(&str, Phone)]) -> String {
        let id = quote_common::unique("CU");
        self.db.customer(&id, "MAIN").await;
        let keys = keys();
        for (raw, how) in phones {
            let value =
                xc_kernel::pii::normalize(IdentityType::Phone, raw, None).expect("a valid phone");
            let sealed = PiiCipher::new(keys.clone())
                .encrypt(format!("{id}PHONE").as_bytes(), value.as_bytes());
            let hash = LookupHasher::new(&keys)
                .hash(IdentityType::Phone, &value)
                .to_vec();
            let (status, verified_at) = match how {
                Phone::ActiveVerified => ("ACTIVE", Some(at("2026-10-01T00:00:00Z"))),
                Phone::ActiveUnverified => ("ACTIVE", None),
                Phone::RevokedVerified => ("REVOKED", Some(at("2026-09-01T00:00:00Z"))),
            };
            sqlx::query(
                "INSERT INTO customer_identities \
                   (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, verified_at, status) \
                 VALUES ($1, $2, $3, 'PHONE', $4, $5, $6, $7, $8)",
            )
            .bind(Uuid::new_v4())
            .bind(org())
            .bind(&id)
            .bind(sealed.bytes)
            .bind(&sealed.key_id)
            .bind(hash)
            .bind(verified_at)
            .bind(status)
            .execute(&self.db.pool)
            .await
            .expect("insert a PHONE identity");
        }
        id
    }

    /// A COMPLETED LIVE BUY of 100 USD at MAIN by `customer`: one line, THB 3210.50, settled 3200.50; with
    /// its receipt revision 1. Answers the facts LY-1 enqueues from.
    async fn exchange(&self, customer: &str) -> Exchange {
        let id = Uuid::new_v4();
        let number = quote_common::unique("XC-MAIN");
        self.db
            .exec(&format!(
                "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
                    customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
                    recorded_by, status, completed_at, organization_id) \
                 VALUES ('{id}', '{number}', 'MAIN', 'LIVE', 'ly2-shift', 'sub-{id}', 'hash', '{customer}', \
                    'Somchai', 'THA', 'EMP-001', 'WALK-IN', '', '2026-10-07', '{COMPLETED_AT}', \
                    '{COMPLETED_AT}', 'owner', 'COMPLETED', '{COMPLETED_AT}', '{ORG_1}')"
            ))
            .await;
        self.db
            .exec(&format!(
                "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                    quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, \
                    created_at, updated_at) \
                 VALUES (gen_random_uuid(), '{ORG_1}', '{id}', 'BUY', 'USD', 100, 1, 32.105, 3210.50, 10.00, \
                    3200.50, 1, '{COMPLETED_AT}', '{COMPLETED_AT}')"
            ))
            .await;
        self.db
            .exec(&format!(
                "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
                    document_ref, kind, created_at, updated_at) \
                 VALUES (gen_random_uuid(), '{ORG_1}', '{id}', '{number}', 1, '{COMPLETED_AT}', \
                    'RECEIPT-LY2', 'RECEIPT', '{COMPLETED_AT}', '{COMPLETED_AT}')"
            ))
            .await;
        let facts = CompletedFacts {
            organization_id: org(),
            transaction_id: id,
            document_ref: number,
            customer_id: Some(customer.to_owned()),
            mode: Mode::Live,
            branch_id: "MAIN".to_owned(),
            completed_at: at(COMPLETED_AT),
            business_date: NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(),
            reversible_until: at("2026-10-07T16:59:59Z"),
            lines: vec![CompletedLine {
                currency: "USD".to_owned(),
                shop_side: Side::Buy,
                quantity: dec("100"),
                unit_size: dec("1"),
                thb_amount: dec("3210.50"),
                settle_thb: dec("3200.50"),
                realized_margin_thb: None,
            }],
        };
        Exchange {
            customer: customer.to_owned(),
            facts,
        }
    }

    /// The earn row, as LY-1 writes it in the completion's transaction.
    async fn enqueue_completed(&self, x: &Exchange) {
        let mut tx = self.db.pool.begin().await.expect("begin");
        LoyaltyEvents::completed_in(&mut tx, &x.facts)
            .await
            .expect("LY-1 completed_in answers Ok");
        tx.commit().await.expect("commit");
    }

    /// A POSTED whole reversal of `x` (RV-3's shape: approved request, posting, receipt revision 2), and
    /// its row as LY-1 writes it. Answers the reversal id.
    async fn reverse(&self, x: &Exchange) -> Uuid {
        let id = Uuid::new_v4();
        let request = Uuid::new_v4();
        let receipt = Uuid::new_v4();
        let tx_id = x.facts.transaction_id;
        let number = &x.facts.document_ref;
        self.db
            .exec(&format!(
                "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
                    object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
                    status, idempotency_key) \
                 VALUES ('{request}', '{ORG_1}', 'TRANSACTION_REVERSAL', 'MAIN', '{id}', 'h', '1', \
                    '{{}}', 'EMP-FRONT', '{COMPLETED_AT}', 'APPROVED', 'k-{request}')"
            ))
            .await;
        self.db
            .exec(&format!(
                "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, \
                    issued_at, document_ref, kind, created_at, updated_at) \
                 VALUES ('{receipt}', '{ORG_1}', '{tx_id}', '{number}-R2', 2, '{POSTED_AT}', \
                    'RECEIPT-LY2', 'REVERSAL', '{POSTED_AT}', '{POSTED_AT}')"
            ))
            .await;
        let posting_id = Uuid::new_v4();
        self.db
            .exec(&format!(
                "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
                 VALUES ('{posting_id}', '{ORG_1}', 'MAIN', 'REVERSAL', '{id}', 'reversal', 'POSTED', '{POSTED_AT}', 'EMP-FRONT')"
            ))
            .await;
        self.db
            .exec(&format!(
                "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
                    reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
                    requested_by, requested_at, posted_by, posted_at) \
                 VALUES ('{id}', '{ORG_1}', 'MAIN', '{tx_id}', 'POSTED', 'wrong currency', 3200.50, \
                    '2026-10-07', '{request}', '{posting_id}', '{receipt}', 'EMP-FRONT', '{COMPLETED_AT}', \
                    'EMP-FRONT', '{POSTED_AT}')"
            ))
            .await;
        let facts = ReversedFacts {
            organization_id: org(),
            reversal_id: id,
            transaction_id: tx_id,
            customer_id: Some(x.customer.clone()),
            posted_at: at(POSTED_AT),
            amount_thb: dec("3200.50"),
            full: true,
        };
        let mut tx = self.db.pool.begin().await.expect("begin");
        LoyaltyEvents::reversed_in(&mut tx, &facts)
            .await
            .expect("LY-1 reversed_in answers Ok");
        tx.commit().await.expect("commit");
        id
    }

    /// `(id, status, attempts, last_http_status, last_error, next_attempt_at > now())` of the row keyed `key`.
    async fn row(&self, key: &str) -> Row {
        let (id, status, attempts, http, error, later, payload): (
            String,
            String,
            i32,
            Option<i32>,
            Option<String>,
            bool,
            Value,
        ) = sqlx::query_as(
            "SELECT id::text, status, attempts, last_http_status, last_error, next_attempt_at > now(), \
                    payload_redacted \
             FROM outbox_events WHERE idempotency_key = $1",
        )
        .bind(key)
        .fetch_one(&self.db.pool)
        .await
        .unwrap_or_else(|e| panic!("the row keyed {key}: {e}"));
        Row {
            id,
            status,
            attempts,
            http,
            error,
            later,
            payload,
        }
    }

    /// Make the row keyed `key` due now (skip its backoff), as the clock would.
    async fn due_now(&self, key: &str) {
        self.db
            .exec(&format!(
                "UPDATE outbox_events SET next_attempt_at = now() - interval '1 second' \
                 WHERE idempotency_key = '{key}'"
            ))
            .await;
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

#[derive(Debug)]
struct Row {
    id: String,
    status: String,
    attempts: i32,
    http: Option<i32>,
    error: Option<String>,
    /// `next_attempt_at` is in the future: the row waits for its backoff.
    later: bool,
    payload: Value,
}

/// The `n`th distinct Thai mobile number: one ACTIVE PHONE per number per organisation
/// (`customer_identities_one_active`), so each customer of a test gets its own.
fn nth_phone(n: usize) -> String {
    format!("08123456{n:02}")
}

fn earn_key(x: &Exchange) -> String {
    format!("exchange:{}", x.facts.transaction_id)
}

fn reversal_key(id: Uuid) -> String {
    format!("reversal:{id}")
}

/// Nothing personal or secret leaked into the stored row (D-2): not the phone in any form, not the secret.
fn assert_row_holds_no_phone_or_secret(row: &Row) {
    let stored = format!("{} {:?}", row.payload, row.error);
    for needle in ["+66", "+1415", "8123456", SECRET] {
        assert!(
            !stored.contains(needle),
            "the stored row holds {needle}: {stored}"
        );
    }
}

// ---------------------------------------------------------------- registration

#[tokio::test]
async fn configured_registers_one_handler_per_loyalty_kind_on_one_destination() {
    let settings = common::settings(&[
        ("XC_LOYALTY_BASE_URL", "http://loyalty.test"),
        ("XC_LOYALTY_TENANT_ID", TENANT),
        ("XC_LOYALTY_WEBHOOK_SECRET", SECRET),
    ]);
    let state = AppState::new(Arc::new(settings), common::unreachable_pool());
    let registry = outbox_handlers::registry(&state).expect("no clash");
    let completed = registry
        .handler("loyalty.exchange_completed")
        .expect("loyalty_completed.rs registers the earn handler");
    let reversed = registry
        .handler("loyalty.exchange_reversed")
        .expect("loyalty_reversed.rs registers the reversal handler");
    assert_eq!(completed.kind().event_type, LOYALTY_COMPLETED.event_type);
    assert_eq!(reversed.kind().event_type, LOYALTY_REVERSED.event_type);
    assert_eq!(completed.kind().destination, LOYALTY_COMPLETED.destination);
    assert_eq!(reversed.kind().destination, LOYALTY_REVERSED.destination);
}

#[tokio::test]
async fn a_missing_or_blank_variable_registers_neither_handler() {
    let cases: [&[(&str, &str)]; 6] = [
        &[],
        &[
            ("XC_LOYALTY_TENANT_ID", TENANT),
            ("XC_LOYALTY_WEBHOOK_SECRET", SECRET),
        ],
        &[
            ("XC_LOYALTY_BASE_URL", "http://loyalty.test"),
            ("XC_LOYALTY_WEBHOOK_SECRET", SECRET),
        ],
        &[
            ("XC_LOYALTY_BASE_URL", "http://loyalty.test"),
            ("XC_LOYALTY_TENANT_ID", TENANT),
        ],
        &[
            ("XC_LOYALTY_BASE_URL", "  "),
            ("XC_LOYALTY_TENANT_ID", TENANT),
            ("XC_LOYALTY_WEBHOOK_SECRET", SECRET),
        ],
        &[
            ("XC_LOYALTY_BASE_URL", "http://loyalty.test"),
            ("XC_LOYALTY_TENANT_ID", TENANT),
            ("XC_LOYALTY_WEBHOOK_SECRET", ""),
        ],
    ];
    for pairs in cases {
        let state = AppState::new(
            Arc::new(common::settings(pairs)),
            common::unreachable_pool(),
        );
        let registry = outbox_handlers::registry(&state).expect("boots without Loyalty");
        for kind in ["loyalty.exchange_completed", "loyalty.exchange_reversed"] {
            assert!(
                registry.handler(kind).is_none(),
                "{pairs:?}: {kind} is not handled; its rows wait and expire after 7 days"
            );
        }
    }
}

// ---------------------------------------------------------------- the earn

#[tokio::test]
async fn an_earn_is_posted_signed_with_its_key_and_the_designs_body_and_turns_delivered() {
    let w = World::start().await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;

    let report = w.tick().await;
    assert_eq!(report.delivered, 1, "{report:?}");

    let seen = w.fake.seen();
    assert_eq!(seen.len(), 1, "exactly one request: {seen:?}");
    let req = &seen[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/webhooks/xchange");
    assert_eq!(req.header("idempotency-key"), earn_key(&x));
    assert!(
        req.header("content-type").starts_with("application/json"),
        "{:?}",
        req.headers
    );
    assert_signed(req);

    let row = w.row(&earn_key(&x)).await;
    let body = req.json();
    assert_eq!(body["id"], json!(row.id), "id is the outbox row's id");
    assert_eq!(body["type"], json!("xchange.exchange_completed.v1"));
    let data = &body["data"];
    assert_eq!(data["tenantId"], json!(TENANT));
    assert_eq!(
        data["phone"],
        json!(PHONE_E164),
        "the verified phone, E.164"
    );
    assert_eq!(
        data["customerRef"],
        json!(LoyaltyCustomerRef::of(org(), &customer).to_string())
    );
    assert_eq!(
        data["transactionId"],
        json!(x.facts.transaction_id.to_string())
    );
    assert_eq!(data["documentRef"], json!(x.facts.document_ref));
    assert_eq!(data["branchId"], json!("MAIN"));
    assert_eq!(instant(&data["completedAt"]), at(COMPLETED_AT));
    assert_eq!(
        data["businessDate"],
        json!("2026-10-07"),
        "the completion date in the branch timezone (Bangkok), not the UTC date"
    );
    let until = instant(&data["reversibleUntil"]).with_timezone(&Bangkok);
    assert!(
        until.date_naive() == NaiveDate::from_ymd_opt(2026, 10, 7).unwrap()
            && until.hour() == 23
            && until.minute() == 59
            || until.date_naive() == NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
                && until.num_seconds_from_midnight() == 0,
        "reversibleUntil is the end of the business day in Bangkok (D-7), got {until}"
    );
    assert_eq!(
        data["lines"],
        json!([{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100.0000",
            "unitSize": "1.0000",
            "thbAmount": "3210.50",
            "settleThb": "3200.50",
            "realizedMarginThb": null,
        }]),
        "money as strings: THB with 2 places, quantity and unit size with 4"
    );

    assert_eq!(row.status, "DELIVERED");
    assert_eq!(row.http, Some(200));
    assert_row_holds_no_phone_or_secret(&row);
    w.finish().await;
}

#[tokio::test]
async fn any_2xx_answer_delivers_the_row() {
    let w = World::start().await;
    let mut keys = Vec::new();
    for (n, code) in [201, 202, 204].into_iter().enumerate() {
        let customer = w.customer(&[(&nth_phone(n), Phone::ActiveVerified)]).await;
        let x = w.exchange(&customer).await;
        w.enqueue_completed(&x).await;
        w.fake.answer(Answer::Status(code));
        keys.push((code, earn_key(&x)));
    }
    let report = w.tick().await;
    assert_eq!(report.delivered, 3, "{report:?}");
    for (code, key) in keys {
        let row = w.row(&key).await;
        assert_eq!(row.status, "DELIVERED", "{code}: {row:?}");
        assert_eq!(row.http, Some(i32::from(code)), "{code}: {row:?}");
    }
    w.finish().await;
}

#[tokio::test]
async fn a_foreign_phone_is_sent_as_its_e164_value() {
    let w = World::start().await;
    let customer = w.customer(&[("+14155550123", Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;
    w.tick().await;
    let seen = w.fake.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].json()["data"]["phone"], json!("+14155550123"));
    w.finish().await;
}

// ---------------------------------------------------------------- the phone at delivery (§7)

#[tokio::test]
async fn the_phone_current_at_delivery_is_sent_not_a_revoked_one() {
    let w = World::start().await;
    let customer = w
        .customer(&[
            ("0899999999", Phone::RevokedVerified),
            (PHONE, Phone::ActiveVerified),
        ])
        .await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;
    w.tick().await;
    let seen = w.fake.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].json()["data"]["phone"], json!(PHONE_E164));
    w.finish().await;
}

#[tokio::test]
async fn no_verified_phone_at_delivery_fails_the_row_with_no_verified_phone_and_sends_nothing() {
    let w = World::start().await;
    let mut keys = Vec::new();
    for (case, phones) in [
        ("no PHONE row", &[][..]),
        ("revoked", &[(PHONE, Phone::RevokedVerified)][..]),
        ("never verified", &[(PHONE, Phone::ActiveUnverified)][..]),
    ] {
        let customer = w.customer(phones).await;
        let x = w.exchange(&customer).await;
        w.enqueue_completed(&x).await;
        keys.push((case, earn_key(&x)));
    }
    let report = w.tick().await;
    assert_eq!(report.failed, 3, "{report:?}");
    assert!(
        w.fake.seen().is_empty(),
        "a phone-less event is never sent: {:?}",
        w.fake.seen()
    );
    for (case, key) in keys {
        let row = w.row(&key).await;
        assert_eq!(row.status, "FAILED", "{case}: {row:?}");
        assert!(
            row.error
                .as_deref()
                .is_some_and(|e| e.contains("noVerifiedPhone")),
            "{case}: the reason is noVerifiedPhone: {row:?}"
        );
        assert_row_holds_no_phone_or_secret(&row);
    }
    w.finish().await;
}

// ---------------------------------------------------------------- Loyalty's answers (§4.5)

#[tokio::test]
async fn a_400_fails_the_row_for_good() {
    let w = World::start().await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;
    w.fake.answer(Answer::Status(400));

    let report = w.tick().await;
    assert_eq!(report.failed, 1, "{report:?}");
    let row = w.row(&earn_key(&x)).await;
    assert_eq!(row.status, "FAILED", "{row:?}");
    assert_eq!(row.http, Some(400));
    assert_row_holds_no_phone_or_secret(&row);

    w.due_now(&earn_key(&x)).await;
    w.tick().await;
    assert_eq!(w.fake.seen().len(), 1, "a FAILED row is never sent again");
    w.finish().await;
}

#[tokio::test]
async fn a_401_a_503_a_500_and_a_timeout_are_retried_later() {
    let w = World::start().await;
    let mut keys = Vec::new();
    for (n, (case, answer)) in [
        ("401", Answer::Status(401)),
        ("503", Answer::Status(503)),
        ("500", Answer::Status(500)),
        ("timeout", Answer::Hang),
    ]
    .into_iter()
    .enumerate()
    {
        let customer = w.customer(&[(&nth_phone(n), Phone::ActiveVerified)]).await;
        let x = w.exchange(&customer).await;
        w.enqueue_completed(&x).await;
        w.fake.answer(answer);
        keys.push((case, earn_key(&x)));
    }
    let report = w.tick().await;
    assert_eq!(report.retried, 4, "{report:?}");
    assert_eq!(w.fake.seen().len(), 4, "each row was sent once");
    for (case, key) in keys {
        let row = w.row(&key).await;
        assert_eq!(row.status, "PENDING", "{case}: {row:?}");
        assert_eq!(row.attempts, 1, "{case}: {row:?}");
        assert!(row.later, "{case}: waits for its backoff: {row:?}");
        assert_row_holds_no_phone_or_secret(&row);
    }
    w.finish().await;
}

#[tokio::test]
async fn an_unreachable_loyalty_is_retried_later() {
    let fake = FakeLoyalty::start().await;
    let closed = format!("http://127.0.0.1:{}", common::closed_port());
    let w = World::with_base_url(fake, &closed).await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;

    let report = w.tick().await;
    assert_eq!(report.retried, 1, "{report:?}");
    let row = w.row(&earn_key(&x)).await;
    assert_eq!(row.status, "PENDING", "{row:?}");
    assert!(row.later, "{row:?}");
    assert_row_holds_no_phone_or_secret(&row);
    w.finish().await;
}

#[tokio::test]
async fn a_retried_earn_is_sent_again_with_the_same_key_and_id() {
    let w = World::start().await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;
    w.fake.answer(Answer::Status(503));
    w.tick().await;
    w.due_now(&earn_key(&x)).await;
    let report = w.tick().await;
    assert_eq!(report.delivered, 1, "{report:?}");

    let seen = w.fake.seen();
    assert_eq!(seen.len(), 2, "{seen:?}");
    for req in &seen {
        assert_eq!(req.header("idempotency-key"), earn_key(&x));
        assert_signed(req);
    }
    assert_eq!(
        seen[0].json()["id"],
        seen[1].json()["id"],
        "Loyalty dedupes a redelivery: the same event id"
    );
    assert_eq!(seen[0].json()["data"], seen[1].json()["data"]);
    let row = w.row(&earn_key(&x)).await;
    assert_eq!(row.status, "DELIVERED");
    assert_eq!(row.attempts, 2);
    w.finish().await;
}

// ---------------------------------------------------------------- the reversal

#[tokio::test]
async fn a_reversal_is_posted_after_its_earn_with_the_cumulative_amount_and_full() {
    let w = World::start().await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;
    let reversal = w.reverse(&x).await;

    w.tick().await;
    w.tick().await;

    let seen = w.fake.seen();
    assert_eq!(seen.len(), 2, "the earn, then the reversal: {seen:?}");
    assert_eq!(seen[0].header("idempotency-key"), earn_key(&x));
    let req = &seen[1];
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/webhooks/xchange");
    assert_eq!(req.header("idempotency-key"), reversal_key(reversal));
    assert_signed(req);

    let row = w.row(&reversal_key(reversal)).await;
    let body = req.json();
    assert_eq!(body["id"], json!(row.id));
    assert_eq!(body["type"], json!("xchange.exchange_reversed.v1"));
    let data = &body["data"];
    assert_eq!(data["tenantId"], json!(TENANT));
    assert_eq!(data["phone"], json!(PHONE_E164));
    assert_eq!(
        data["customerRef"],
        json!(LoyaltyCustomerRef::of(org(), &customer).to_string())
    );
    assert_eq!(
        data["transactionId"],
        json!(x.facts.transaction_id.to_string()),
        "the original transaction"
    );
    assert_eq!(data["reversalId"], json!(reversal.to_string()));
    assert_eq!(instant(&data["postedAt"]), at(POSTED_AT));
    assert_eq!(
        data["amountThb"],
        json!("3200.50"),
        "cumulative reversed THB for the transaction, a 2-place string"
    );
    assert_eq!(data["full"], json!(true));

    assert_eq!(row.status, "DELIVERED");
    assert_row_holds_no_phone_or_secret(&row);
    w.finish().await;
}

#[tokio::test]
async fn a_reversal_waits_while_its_earn_is_pending_in_backoff() {
    let w = World::start().await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;
    let reversal = w.reverse(&x).await;
    w.fake.answer(Answer::Status(503));

    w.tick().await;
    w.tick().await;
    let seen = w.fake.seen();
    assert_eq!(seen.len(), 1, "only the earn was tried: {seen:?}");
    assert_eq!(seen[0].header("idempotency-key"), earn_key(&x));
    let held = w.row(&reversal_key(reversal)).await;
    assert_eq!(held.status, "PENDING", "{held:?}");
    assert_eq!(
        held.attempts, 0,
        "never tried while the earn waits: {held:?}"
    );

    w.due_now(&earn_key(&x)).await;
    w.tick().await;
    w.tick().await;
    let keys: Vec<String> = w
        .fake
        .seen()
        .iter()
        .map(|s| s.header("idempotency-key").to_owned())
        .collect();
    assert_eq!(
        keys,
        vec![earn_key(&x), earn_key(&x), reversal_key(reversal)],
        "per-aggregate FIFO: the reversal goes only after its earn"
    );
    assert_eq!(w.row(&reversal_key(reversal)).await.status, "DELIVERED");
    w.finish().await;
}

#[tokio::test]
async fn a_reversal_whose_customer_lost_the_verified_phone_fails_with_no_verified_phone() {
    let w = World::start().await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;
    w.tick().await;
    assert_eq!(w.row(&earn_key(&x)).await.status, "DELIVERED");

    // the phone is revoked between the earn and the reversal (design §7): the phone is read at delivery
    w.db.exec(&format!(
        "UPDATE customer_identities SET status = 'REVOKED' WHERE customer_id = '{customer}'"
    ))
    .await;
    let reversal = w.reverse(&x).await;
    let report = w.tick().await;
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(
        w.fake.seen().len(),
        1,
        "only the earn was ever sent; a phone-less reversal never is"
    );
    let row = w.row(&reversal_key(reversal)).await;
    assert_eq!(row.status, "FAILED", "{row:?}");
    assert!(
        row.error
            .as_deref()
            .is_some_and(|e| e.contains("noVerifiedPhone")),
        "the reason is noVerifiedPhone: {row:?}"
    );
    assert_row_holds_no_phone_or_secret(&row);
    w.finish().await;
}

// ---------------------------------------------------------------- branch timezone reversibleUntil (#2202)

#[tokio::test]
async fn an_earn_at_an_asia_tokyo_branch_sets_reversible_until_to_end_of_tokyo_business_day() {
    let w = World::start().await;
    w.db.exec("UPDATE branches SET timezone = 'Asia/Tokyo' WHERE id = 'MAIN'")
        .await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;

    let report = w.tick().await;
    assert_eq!(report.delivered, 1, "{report:?}");

    let seen = w.fake.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let data = &seen[0].json()["data"];
    assert_eq!(data["businessDate"], json!("2026-10-07"));
    assert_eq!(instant(&data["completedAt"]), at(COMPLETED_AT));
    assert_eq!(
        instant(&data["reversibleUntil"]),
        at("2026-10-07T15:00:00Z"),
        "end of 2026-10-07 in Asia/Tokyo (UTC+9) is 2026-10-07T15:00:00Z"
    );
    w.finish().await;
}

#[tokio::test]
async fn an_earn_at_a_europe_london_branch_sets_reversible_until_to_end_of_london_business_day() {
    let w = World::start().await;
    w.db.exec("UPDATE branches SET timezone = 'Europe/London' WHERE id = 'MAIN'")
        .await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.enqueue_completed(&x).await;

    let report = w.tick().await;
    assert_eq!(report.delivered, 1, "{report:?}");

    let seen = w.fake.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let data = &seen[0].json()["data"];
    assert_eq!(data["businessDate"], json!("2026-10-07"));
    assert_eq!(instant(&data["completedAt"]), at(COMPLETED_AT));
    assert_eq!(
        instant(&data["reversibleUntil"]),
        at("2026-10-07T23:00:00Z"),
        "end of 2026-10-07 in Europe/London (BST, UTC+1) is 2026-10-07T23:00:00Z"
    );
    w.finish().await;
}

#[tokio::test]
async fn an_earn_in_a_zone_with_a_dst_transition_at_midnight_uses_earliest_instant_without_panic() {
    let w = World::start().await;
    // America/Havana transitions at local midnight (00:00); on 2026-11-01 00:00:00 local is ambiguous (-04:00 then -05:00).
    w.db.exec("UPDATE branches SET timezone = 'America/Havana' WHERE id = 'MAIN'")
        .await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;
    w.db.exec(&format!(
        "UPDATE transactions SET value_date = '2026-10-31' WHERE id = '{}'",
        x.facts.transaction_id
    ))
    .await;
    w.enqueue_completed(&x).await;

    let report = w.tick().await;
    assert_eq!(report.delivered, 1, "{report:?}");

    let seen = w.fake.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let data = &seen[0].json()["data"];
    assert_eq!(data["businessDate"], json!("2026-10-31"));
    assert_eq!(
        instant(&data["reversibleUntil"]),
        at("2026-11-01T04:00:00Z"),
        "end of 2026-10-31 in America/Havana uses the earliest 2026-11-01 00:00 local instant (UTC-4 = 04:00Z)"
    );
    w.finish().await;
}

#[tokio::test]
async fn completed_with_an_unparsable_branch_timezone_returns_internal_error() {
    use xc_loyalty::domain::facts_source::LoyaltyFactsSource;
    use xchange::infrastructure::loyalty::pg_facts_source::PgLoyaltyFactsSource;

    let w = World::start().await;
    w.db.exec("UPDATE branches SET timezone = 'Not/A_Zone' WHERE id = 'MAIN'")
        .await;
    let customer = w.customer(&[(PHONE, Phone::ActiveVerified)]).await;
    let x = w.exchange(&customer).await;

    let src = PgLoyaltyFactsSource::new(w.db.pool.clone());
    let err = src
        .completed(x.facts.transaction_id)
        .await
        .expect_err("unparsable branch timezone must return Err");
    assert!(
        matches!(&err, pos_error::AppError::Internal(msg) if msg == "branch timezone Not/A_Zone"),
        "expected AppError::Internal(\"branch timezone Not/A_Zone\"), got {err:?}"
    );
    w.finish().await;
}
