//! RA-2 (#980, design `docs/design/blueprint/broker-seam.md` §5.4) — `BrokerAttributionHandler`
//! against a fake Broker on `127.0.0.1:0`, driven through OB-1's real worker and store.
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Each test captures a code with a customer sub through RA-1's `ReferralCaptureService`, which
//! writes the `broker.attribution` outbox row, then runs one `OutboxWorker::tick` with only this
//! handler registered. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod member_phone_seeds;

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use common::{MAIN, TestDb, at};
use member_phone_seeds::{MEMBER_E164, TYPED_E164};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_outbox::application::registry::OutboxRegistry;
use xc_outbox::application::worker::OutboxWorker;
use xc_outbox::domain::handler::OutboxHandler;
use xc_outbox::infrastructure::pg_store::PgOutboxStore;
use xc_referral::application::capture_service::{CaptureInput, ReferralCaptureService};
use xc_referral::domain::capture::{CaptureId, CaptureSource, CaptureTarget};
use xc_referral::domain::events::BROKER_ATTRIBUTION;
use xc_referral::domain::referral_code::ReferralCode;
use xc_referral::infrastructure::booking_member_keys::BookingMemberKeys;
use xc_referral::infrastructure::broker_attribution_handler::{BrokerAttributionHandler, PRODUCT};
use xc_referral::infrastructure::capture_repository::PgCaptureRepository;

const TOKEN: &str = "tok-SECRET-9f2c71e4b0d84a6d";

// ---------------------------------------------------------------- the fake Broker

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Clone)]
enum Answer {
    Status(u16, String),
    /// Accept the request and never answer.
    Hang,
}

#[derive(Clone)]
struct FakeBroker {
    url: String,
    answers: Arc<Mutex<VecDeque<Answer>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeBroker {
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
                    let answer = a
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or(Answer::Status(201, "{}".to_owned()));
                    match answer {
                        Answer::Hang => tokio::time::sleep(Duration::from_secs(60)).await,
                        Answer::Status(code, body) => {
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

// ---------------------------------------------------------------- captured logs

/// A `tracing` subscriber that keeps every event and span field as text.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<String>>);

struct Fields<'a>(&'a mut String);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, " {}={:?}", field.name(), value);
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let _ = write!(self.0, " {}={}", field.name(), value);
    }
}

impl tracing::Subscriber for Logs {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut out = self.0.lock().unwrap();
        span.record(&mut Fields(&mut out));
        out.push('\n');
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut out = self.0.lock().unwrap();
        values.record(&mut Fields(&mut out));
        out.push('\n');
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut out = self.0.lock().unwrap();
        event.record(&mut Fields(&mut out));
        out.push('\n');
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// One process-wide capture, installed before any handler runs. A thread-local
/// `set_default` misses events when other test threads register the same call sites at the
/// same time, so every test's log lines land here, and the token must be in none of them.
fn logs() -> &'static Logs {
    static LOGS: OnceLock<Logs> = OnceLock::new();
    LOGS.get_or_init(|| {
        let logs = Logs::default();
        tracing::subscriber::set_global_default(logs.clone())
            .expect("no other global subscriber in this test binary");
        logs
    })
}

// ---------------------------------------------------------------- helpers

fn handler(db: &TestDb, base_url: &str) -> Arc<BrokerAttributionHandler> {
    logs();
    Arc::new(BrokerAttributionHandler::new(
        db.pool.clone(),
        base_url.to_owned(),
        TOKEN.to_owned(),
        Arc::new(BookingMemberKeys),
    ))
}

fn worker(db: &TestDb, h: Arc<BrokerAttributionHandler>) -> OutboxWorker {
    let h: Arc<dyn OutboxHandler> = h;
    OutboxWorker::new(
        Arc::new(PgOutboxStore::new(db.pool.clone())),
        Arc::new(OutboxRegistry::new(vec![h]).expect("one handler")),
    )
}

/// A customer, an exchange of theirs, and a capture of `AB12CD` on it with sub `sub-1`.
async fn captured(db: &TestDb) -> (CaptureId, Uuid, String) {
    db.seed_customer("C-1").await;
    let (tx_id, document_ref) = db.seed_transaction("C-1").await;
    let service = ReferralCaptureService::new(
        Arc::new(PgCaptureRepository::new()),
        Arc::new(PgAuditLog::new(db.pool.clone())),
    );
    let mut tx = db.begin().await;
    let id = service
        .capture_in(
            &mut tx,
            CaptureInput {
                code: ReferralCode::parse("ab12cd").expect("a code"),
                on: CaptureTarget::Transaction(tx_id),
                source: CaptureSource::Desk,
                customer_sub: Some("sub-1".to_owned()),
                by: "teller-1".to_owned(),
                at: at(0),
            },
        )
        .await
        .expect("captured with a sub: one outbox row");
    tx.commit().await.expect("commit");
    (id, tx_id, document_ref)
}

/// The outbox row of `capture` and the capture's `broker_outcome`.
async fn state(db: &TestDb, capture: &CaptureId) -> (String, i32, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT o.status, o.attempts, o.last_error, r.broker_outcome \
         FROM outbox_events o JOIN referral_attributions r ON r.id::text = o.aggregate_id \
         WHERE o.aggregate_id = $1",
    )
    .bind(capture.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("the outbox row and its capture")
}

/// Run one tick against a fake Broker that answers `answer`; return the row's state.
async fn deliver_once(answer: Answer) -> (String, i32, Option<String>, Option<String>, Vec<Seen>) {
    let db = TestDb::start().await;
    let (id, _, _) = captured(&db).await;
    let broker = FakeBroker::start().await;
    broker.answer(answer);
    worker(&db, handler(&db, &broker.url))
        .tick()
        .await
        .expect("a tick");
    let (status, attempts, error, outcome) = state(&db, &id).await;
    db.finish().await;
    (status, attempts, error, outcome, broker.seen())
}

// ---------------------------------------------------------------- the request

#[tokio::test]
async fn the_request_is_brokers_attribution_call_with_the_rows_key() {
    let db = TestDb::start().await;
    let (id, _, _) = captured(&db).await;
    let broker = FakeBroker::start().await;
    broker.answer(Answer::Status(201, "{}".to_owned()));
    let h = handler(&db, &broker.url);
    assert_eq!(h.kind(), &BROKER_ATTRIBUTION);
    worker(&db, h).tick().await.expect("a tick");

    let seen = broker.seen();
    assert_eq!(seen.len(), 1, "sent exactly once");
    let r = &seen[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/api/v1/attributions");
    assert_eq!(
        r.headers.get("authorization").map(String::as_str),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert_eq!(
        r.headers.get("idempotency-key").map(String::as_str),
        Some(format!("attr:{id}").as_str()),
        "the outbox row's key"
    );
    assert!(
        r.headers
            .get("content-type")
            .is_some_and(|v| v.starts_with("application/json")),
        "{:?}",
        r.headers
    );
    let body: Value = serde_json::from_str(&r.body).expect("a JSON body");
    assert_eq!(
        body,
        json!({ "code": "AB12CD", "product": PRODUCT, "customerSub": "sub-1" }),
        "exactly these keys; no phone without a verified source"
    );
    assert_eq!(PRODUCT, "xchange-fx");
    db.finish().await;
}

/// Re-pinned for BK-10 #1412 by claude/29df2ec5 (lane controller ruling 1, issuecomment-6140233542):
/// the member is the booking's customer with a verified ACTIVE `PHONE` identity, sent in E.164
/// (ruling 3); the phone typed at the desk is the customer's unverified identity
/// (`ContactReserve`), which never leaves v3.
#[tokio::test]
async fn a_verified_members_phone_is_sent_and_a_typed_booking_phone_is_not() {
    for member in [true, false] {
        let db = TestDb::start().await;
        let (id, tx_id, _) = captured(&db).await;
        if member {
            member_phone_seeds::seed_phone(&db.pool, "C-1", MEMBER_E164, true, "ACTIVE").await;
        } else {
            member_phone_seeds::seed_phone(&db.pool, "C-1", TYPED_E164, false, "ACTIVE").await;
        }
        member_phone_seeds::seed_completed_booking(&db.pool, MAIN, "C-1", tx_id).await;
        let broker = FakeBroker::start().await;
        worker(&db, handler(&db, &broker.url))
            .tick()
            .await
            .expect("a tick");
        let body: Value = serde_json::from_str(&broker.seen()[0].body).expect("JSON");
        if member {
            assert_eq!(
                body,
                json!({ "code": "AB12CD", "product": PRODUCT, "customerSub": "sub-1", "phone": MEMBER_E164 })
            );
        } else {
            assert_eq!(
                body,
                json!({ "code": "AB12CD", "product": PRODUCT, "customerSub": "sub-1" }),
                "a typed booking phone never leaves v3 (D-6)"
            );
        }
        assert_eq!(state(&db, &id).await.0, "DELIVERED");
        db.finish().await;
    }
}

// ---------------------------------------------------------------- Broker's answers

#[tokio::test]
async fn created_and_existing_are_delivered() {
    for (code, outcome) in [(201, "CREATED"), (200, "EXISTING")] {
        let (status, _, _, got, seen) = deliver_once(Answer::Status(code, "{}".to_owned())).await;
        assert_eq!(seen.len(), 1);
        assert_eq!(status, "DELIVERED", "{code}");
        assert_eq!(got.as_deref(), Some(outcome), "{code}");
    }
}

#[tokio::test]
async fn brokers_final_refusals_are_failed_with_their_outcome() {
    let cases = [
        (404, r#"{"error":"unknown_code"}"#, "UNKNOWN_CODE"),
        (409, r#"{"error":"self_referral"}"#, "SELF_REFERRAL"),
        (409, r#"{"error":"held_by_other_broker"}"#, "HELD_BY_OTHER"),
        (409, r#"{"error":"something_else"}"#, "REFUSED"),
        (400, r#"{"error":"bad_request"}"#, "REFUSED"),
        (403, "{}", "REFUSED"),
        (422, "{}", "REFUSED"),
    ];
    for (code, body, outcome) in cases {
        let (status, attempts, _, got, seen) =
            deliver_once(Answer::Status(code, body.to_owned())).await;
        assert_eq!(seen.len(), 1);
        assert_eq!(status, "FAILED", "{code} {body}");
        assert_eq!(got.as_deref(), Some(outcome), "{code} {body}");
        assert_eq!(attempts, 1, "{code}: a final answer is not retried");
    }
}

#[tokio::test]
async fn retryable_answers_leave_the_row_pending_with_no_outcome() {
    for code in [401, 408, 429, 500, 502, 503] {
        let (status, attempts, _, got, seen) =
            deliver_once(Answer::Status(code, "{}".to_owned())).await;
        assert_eq!(seen.len(), 1);
        assert_eq!(status, "PENDING", "{code} is retried");
        assert_eq!(attempts, 1, "{code}");
        assert_eq!(got, None, "{code}: no outcome yet");
    }
}

#[tokio::test]
async fn an_unreachable_broker_is_retried() {
    let db = TestDb::start().await;
    let (id, _, _) = captured(&db).await;
    // a port nothing listens on
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr");
    worker(&db, handler(&db, &format!("http://{closed}")))
        .tick()
        .await
        .expect("a tick");
    let (status, _, _, outcome) = state(&db, &id).await;
    assert_eq!(status, "PENDING");
    assert_eq!(outcome, None);
    db.finish().await;
}

#[tokio::test]
async fn a_broker_that_never_answers_times_out_after_ten_seconds_and_is_retried() {
    let db = TestDb::start().await;
    let (id, _, _) = captured(&db).await;
    let broker = FakeBroker::start().await;
    broker.answer(Answer::Hang);
    let started = Instant::now();
    worker(&db, handler(&db, &broker.url))
        .tick()
        .await
        .expect("a tick");
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(9) && took < Duration::from_secs(20),
        "a 10 s timeout, took {took:?}"
    );
    let (status, _, _, outcome) = state(&db, &id).await;
    assert_eq!(status, "PENDING");
    assert_eq!(outcome, None);
    db.finish().await;
}

#[tokio::test]
async fn a_redelivery_carries_the_same_idempotency_key() {
    let db = TestDb::start().await;
    let (id, _, _) = captured(&db).await;
    let broker = FakeBroker::start().await;
    broker.answer(Answer::Status(503, "{}".to_owned()));
    broker.answer(Answer::Status(201, "{}".to_owned()));
    let w = worker(&db, handler(&db, &broker.url));
    w.tick().await.expect("first tick");
    sqlx::query("UPDATE outbox_events SET next_attempt_at = now() - interval '1 second' WHERE aggregate_id = $1")
        .bind(id.to_string())
        .execute(&db.pool)
        .await
        .expect("make the row due again");
    w.tick().await.expect("second tick");

    let keys: Vec<Option<String>> = broker
        .seen()
        .iter()
        .map(|r| r.headers.get("idempotency-key").cloned())
        .collect();
    let want = Some(format!("attr:{id}"));
    assert_eq!(keys, vec![want.clone(), want]);
    let (status, attempts, _, outcome) = state(&db, &id).await;
    assert_eq!((status.as_str(), attempts), ("DELIVERED", 2));
    assert_eq!(outcome.as_deref(), Some("CREATED"));
    db.finish().await;
}

#[tokio::test]
async fn an_expired_row_is_not_sent_and_records_expired() {
    let db = TestDb::start().await;
    let (id, _, _) = captured(&db).await;
    sqlx::query(
        "UPDATE outbox_events SET created_at = now() - interval '8 days' WHERE aggregate_id = $1",
    )
    .bind(id.to_string())
    .execute(&db.pool)
    .await
    .expect("age the row");
    let broker = FakeBroker::start().await;
    worker(&db, handler(&db, &broker.url))
        .tick()
        .await
        .expect("a tick");
    assert!(broker.seen().is_empty(), "nothing is sent after 7 days");
    let (status, _, _, outcome) = state(&db, &id).await;
    assert_eq!(status, "EXPIRED");
    assert_eq!(outcome.as_deref(), Some("EXPIRED"));
    db.finish().await;
}

// ---------------------------------------------------------------- the token stays secret

#[tokio::test]
async fn the_token_is_never_logged_stored_or_printed() {
    let db = TestDb::start().await;
    let (id, _, _) = captured(&db).await;
    let broker = FakeBroker::start().await;
    for code in [401, 500] {
        broker.answer(Answer::Status(code, r#"{"error":"x"}"#.to_owned()));
    }
    let h = handler(&db, &broker.url);
    assert!(
        !format!("{h:?}").contains(TOKEN),
        "the handler's Debug never prints the token"
    );
    let w = worker(&db, h);
    for code in [401, 500] {
        w.tick().await.expect("a tick");
        // each attempt overwrites `last_error`, so it is read after every one
        let (_, _, last_error, _) = state(&db, &id).await;
        assert!(
            !last_error.unwrap_or_default().contains(TOKEN),
            "{code}: outbox_events.last_error never holds the token"
        );
        sqlx::query("UPDATE outbox_events SET next_attempt_at = now() - interval '1 second' WHERE aggregate_id = $1")
            .bind(id.to_string())
            .execute(&db.pool)
            .await
            .expect("make the row due again");
    }
    // and a network error, whose error text names the URL
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr");
    worker(&db, handler(&db, &format!("http://{closed}")))
        .tick()
        .await
        .expect("a tick");

    let (_, attempts, last_error, _) = state(&db, &id).await;
    assert_eq!(attempts, 3, "three attempts were made");
    assert!(
        !last_error.unwrap_or_default().contains(TOKEN),
        "outbox_events.last_error never holds the token"
    );
    let text = logs().0.lock().unwrap().clone();
    assert!(
        !text.contains(TOKEN),
        "no log line holds the token:\n{text}"
    );
    db.finish().await;
}
