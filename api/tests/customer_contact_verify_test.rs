//! BK-10 (#1412): `customerContactVerify` and the member flag's three readers moved off `members`
//! (booking-slots design `docs/design/blueprint/booking-slots.md` §6 `members` row, §9 BK-10,
//! §9.1 BK-10).
//!
//! Written before the implementation by the ticket author (claude/29df2ec5). Do not edit, delete
//! or weaken these tests; add your own in another file.
//!
//! Locks §9.1 BK-10 and the lane controller's rulings on #1412 (claude/ad05bf8d,
//! issuecomment-6138038554):
//! - verifying a phone writes one ACTIVE `PHONE` identity with `verified_at` (sealed and hashed
//!   through `xc-kernel::pii`, K3) and one audit row that never holds the phone;
//! - the member flag = the customer has an ACTIVE `PHONE` identity with `verified_at` set,
//!   company-wide (PR 639 K1): true at every branch of the organisation in all three readers
//!   (`admin_customers`, the compat snapshot's `members` source, Broker's `BookingMemberKeys`);
//!   a `members` row no longer counts;
//! - (a) `customerContactVerify` is gated by `CustomerHoldPlace` (Teller, BranchManager); refused
//!   ⇒ `400 roleNotAllowed`;
//! - (b) `BookingMemberKeys` reads the Blueprint booking (`bookings.transaction_id` →
//!   `customer_id`) and that customer's ACTIVE verified `PHONE` identity, decrypted; none ⇒ no
//!   key, not an error;
//! - (c) verifying the same phone twice keeps one identity and its first `verified_at`;
//! - (d) a phone ACTIVE for another customer of the organisation ⇒ `400 validationFailed` on
//!   `phone`, nothing written.
//!
//! Round 2, rulings 3–4 on #1412 (issuecomment-6140233542): `keys.phone` is E.164; (a) a verify
//! with a different number revokes the old `PHONE` identity, one ACTIVE `PHONE` per customer;
//! (b) a verify moves the `admin_customers` version of every branch of the organisation.
//!
//! Round 3, ruling 2 on #1412 (issuecomment-6142020662): the every-branch bump must not take
//! branch locks out of the MAIN-first order, pinned by two concurrent verifies at two branches.
//!
//! The identity is sealed as every `PHONE` identity on `main` is (`ContactReserve`,
//! `PhoneVerificationService::confirm_in_tx`): the E.164 form, AAD `"{customer_id}PHONE"`, and
//! `lookup_hash = LookupHasher::hash(IdentityType::Phone, e164)`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it,
//! it does not skip. Every test runs in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, TimeZone, Utc};
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_customer::application::customer::contact_verify::{ContactVerify, ContactVerifyError};
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys, Sealed};
use xc_referral::domain::customer_keys::{CustomerKeys, FeedCustomerKeys};
use xc_referral::infrastructure::booking_member_keys::BookingMemberKeys;
use xchange::application::compat::snapshot_sources::{MemberMark, SnapshotSources};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::compat::snapshot_sources::ServiceSnapshotSources;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// The phone the tests verify, as staff type it, and the forms it must never be stored or
/// answered in.
const TYPED: &str = "081-234-5678";
const E164: &str = "+66812345678";
const NATIONAL: &str = "0812345678";
const SUBSCRIBER: &str = "812345678";
const LAST4: &str = "5678";
/// A second phone, for the idempotency-conflict body.
const OTHER_TYPED: &str = "0899999999";

const MAIN: &str = "MAIN";
const B2: &str = "B2";
/// A branch of the other organisation.
const OTHER_BRANCH: &str = "OTHER1";

const HOST: &str = "localhost:8080";
const SAME_ORIGIN: &str = "http://localhost:8080";
const AS: &str = "x-test-as";
const VERIFY: &str = "/api/v3/commands/customer_contact_verify";

fn test_keys() -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(TEST_KEYS).expect("the test PII key parses"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Unique per process and per call: nextest runs every test in a process of its own.
fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn xc_org() -> Uuid {
    Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("org 1")
}

/// Whole seconds, so a round trip through `timestamptz` is exact on every platform.
fn at(secs: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 13, 3, 0, 0).unwrap() + Duration::seconds(secs)
}

fn e164_hash() -> Vec<u8> {
    LookupHasher::new(&test_keys())
        .hash(IdentityType::Phone, E164)
        .to_vec()
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
struct IdentityRow {
    id: Uuid,
    organization_id: Uuid,
    r#type: String,
    encrypted_value: Vec<u8>,
    key_id: String,
    lookup_hash: Vec<u8>,
    verified_at: Option<DateTime<Utc>>,
    status: String,
}

// ── Test database ────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
    schema: String,
}

impl Db {
    async fn start() -> Self {
        let raw_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL must be set and name a database ending in `_test`.\n\
                 Never touch port 6381."
            )
        });
        assert!(!raw_url.contains(":6381"), "never touch port 6381");
        let options = PgConnectOptions::from_str(&raw_url).expect("valid connection URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("bk10");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("migrations")
                .as_path(),
        )
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("apply api/migrations");

        let _ = xc_customer::install_pii_keys(test_keys());
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, 'Second', $2)")
            .bind(B2)
            .bind(xc_org())
            .execute(&pool)
            .await
            .expect("insert B2");
        Db { pool, schema }
    }

    async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked in start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("reconnect");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }

    fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    /// A second organisation with one branch, `OTHER_BRANCH`.
    async fn other_org(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1, $2, 'Other Org Co.', 'Asia/Bangkok', now())",
        )
        .bind(id)
        .bind(unique("other"))
        .execute(&self.pool)
        .await
        .expect("insert the other organisation");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $2, $3)")
            .bind(OTHER_BRANCH)
            .bind(unique("Other branch"))
            .bind(id)
            .execute(&self.pool)
            .await
            .expect("insert the other organisation's branch");
        id
    }

    /// A LIVE customer of `org`.
    async fn customer(&self, org: Uuid) -> String {
        let id = unique("CX");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2, $1, $3, 'LIVE', $4)",
        )
        .bind(&id)
        .bind(org)
        .bind(format!("Customer {id}"))
        .bind(at(-3600))
        .execute(&self.pool)
        .await
        .expect("insert a customer");
        id
    }

    /// A `PHONE` identity of `customer_id`, sealed as `main` seals them.
    async fn seed_phone(
        &self,
        org: Uuid,
        customer_id: &str,
        e164: &str,
        verified_at: Option<DateTime<Utc>>,
        status: &str,
    ) -> Uuid {
        let keys = test_keys();
        let sealed = PiiCipher::new(keys.clone())
            .encrypt(format!("{customer_id}PHONE").as_bytes(), e164.as_bytes());
        let lookup_hash = LookupHasher::new(&keys).hash(IdentityType::Phone, e164);
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO customer_identities ( \
                 id, customer_id, organization_id, type, encrypted_value, key_id, \
                 lookup_hash, verified_at, status, created_at \
             ) VALUES ($1, $2, $3, 'PHONE', $4, $5, $6, $7, $8, $9)",
        )
        .bind(id)
        .bind(customer_id)
        .bind(org)
        .bind(&sealed.bytes)
        .bind(&sealed.key_id)
        .bind(&lookup_hash[..])
        .bind(verified_at)
        .bind(status)
        .bind(at(-1800))
        .execute(&self.pool)
        .await
        .expect("seed a PHONE identity");
        id
    }

    /// A v2 `members` row of `branch` for `customer_id` (staff-verified, BK-11 drops the table).
    async fn seed_member(&self, branch: &str, customer_id: &str) {
        sqlx::query(
            "INSERT INTO members (id, branch_id, customer_id, name, phone, token_hash, verified_at, verified_by) \
             VALUES ($1, $2, $3, 'Name', $4, $5, $6, 'owner')",
        )
        .bind(unique("MB"))
        .bind(branch)
        .bind(customer_id)
        .bind(NATIONAL)
        .bind(unique("hash"))
        .bind(at(-600))
        .execute(&self.pool)
        .await
        .expect("insert into members");
    }

    async fn phone_identities(&self, customer_id: &str) -> Vec<IdentityRow> {
        sqlx::query_as(
            "SELECT id, organization_id, type, encrypted_value, key_id, lookup_hash, verified_at, status \
             FROM customer_identities WHERE customer_id = $1 AND type = 'PHONE' ORDER BY created_at, id",
        )
        .bind(customer_id)
        .fetch_all(&self.pool)
        .await
        .expect("read customer_identities")
    }

    async fn audit_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM audit_events")
            .fetch_one(&self.pool)
            .await
            .expect("count audit_events")
    }

    /// Every audit row as text, for the "never holds the phone" checks.
    async fn audit_text(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT concat_ws(' | ', actor_id, actor_role, scope, branch_id, action, object_ref, \
                    before_redacted::text, after_redacted::text) \
             FROM audit_events ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read audit_events")
    }

    /// A stored DEMO exchange of `customer_id` at MAIN; answers its id.
    async fn seed_transaction(&self, customer_id: &str) -> (Uuid, String) {
        let document_ref = unique("DOC");
        let id: String = sqlx::query_scalar(
            "INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, \
                 customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
                 bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
                 value_date, paid_at, recorded_at, recorded_by, completed_at) \
             VALUES ($1, $2, 'DEMO', $3, 'payload-hash', $4, 'Customer', 'THA', 'SELL', 'USD', \
                 1000.50, 32.6, 32616.30, 0, 0, 32616.30, 0, 'Cash', '', '', 'owner', '', '', \
                 DATE '2026-10-13', $5, $5, 'owner', $5) RETURNING id::text",
        )
        .bind(&document_ref)
        .bind(MAIN)
        .bind(format!("submission-of-{document_ref}"))
        .bind(customer_id)
        .bind(at(0))
        .fetch_one(&self.pool)
        .await
        .expect("insert into transactions");
        (Uuid::parse_str(&id).expect("a uuid"), document_ref)
    }

    /// A Blueprint booking of `customer_id` at MAIN, COMPLETED by `transaction_id`.
    async fn seed_completed_booking(&self, customer_id: &str, transaction_id: Uuid) {
        let slot = Uuid::new_v4();
        let n = NEXT.fetch_add(1, Ordering::Relaxed) as i64;
        let start = at(86_400 + n * 3600);
        sqlx::query(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, \
                 capacity, reserved, status) \
             VALUES ($1, $2, $3, $4, $5, $6, 4, 1, 'OPEN')",
        )
        .bind(slot)
        .bind(xc_org())
        .bind(MAIN)
        .bind(start.date_naive())
        .bind(start)
        .bind(start + Duration::minutes(30))
        .execute(&self.pool)
        .await
        .expect("insert a booking slot");
        let id = format!("BK-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, \
                 shop_side, requested_currency, requested_quantity, source_channel, status, \
                 qr_token_hash, transaction_id, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, 'SELL', 'USD', 100, 'DESK', 'COMPLETED', $7, $8, 'test')",
        )
        .bind(&id)
        .bind(xc_org())
        .bind(MAIN)
        .bind(unique("B261013"))
        .bind(customer_id)
        .bind(slot)
        .bind(Uuid::new_v4().as_bytes().to_vec())
        .bind(transaction_id)
        .execute(&self.pool)
        .await
        .expect("insert a Blueprint booking");
    }

    /// A v2 booking (`v2_bookings`, renamed by M1) that became `document_ref`, made by a v2 member.
    async fn seed_v2_member_booking(&self, member_id: &str, document_ref: &str) {
        sqlx::query(
            "INSERT INTO v2_bookings (id, branch_id, request_id, request_hash, name, phone, member_id, \
                 type, code, amount, rate, status, created_at, created_by, expires_at, submission_id, \
                 transaction_ref) \
             VALUES ($1, $2, $3, 'h', 'Name', '0899999999', $4, 'SELL', 'USD', 100, 32.6, 'USED', \
                 now(), 'owner', now() + interval '1 hour', $5, $6)",
        )
        .bind(unique("BK"))
        .bind(MAIN)
        .bind(unique("req"))
        .bind(member_id)
        .bind(unique("sub"))
        .bind(document_ref)
        .execute(&self.pool)
        .await
        .expect("insert into v2_bookings");
    }

    async fn member_id_of(&self, customer_id: &str) -> String {
        sqlx::query_scalar("SELECT id FROM members WHERE customer_id = $1 LIMIT 1")
            .bind(customer_id)
            .fetch_one(&self.pool)
            .await
            .expect("the member's id")
    }

    async fn keys_of(&self, transaction_id: Uuid) -> CustomerKeys {
        let mut tx = self.pool.begin().await.expect("begin");
        let keys = BookingMemberKeys
            .keys_in(&mut tx, transaction_id)
            .await
            .unwrap_or_else(|e| panic!("keys_in must not fail for {transaction_id}: {e:?}"));
        drop(tx);
        keys
    }

    /// The compat snapshot's member marks of `branch`, in a fixed order.
    async fn snapshot_marks(&self, branch: &str) -> Vec<MemberMark> {
        let sources = ServiceSnapshotSources::new(
            TransactionService::new(
                Arc::new(PgTransactionRepository::new(self.pool.clone())),
                self.branch_tx(),
            ),
            AccountService::new(
                Arc::new(PgAccountRepository::new(self.pool.clone())),
                PasswordHasher::with_cost(8, 1, 1),
            ),
        );
        let mut marks = sources
            .members(&BranchId::new(branch))
            .await
            .unwrap_or_else(|e| panic!("the snapshot's members of {branch}: {e:?}"));
        marks.sort_by(|a, b| a.customer_id.cmp(&b.customer_id));
        marks
    }

    /// Runs `ContactVerify::verify_in` as a Teller of MAIN in a transaction of its own, committed
    /// when it answers `Ok`.
    async fn verify(
        &self,
        customer_id: &str,
        phone: &str,
        now: DateTime<Utc>,
    ) -> Result<
        xc_customer::application::customer::contact_verify::ContactSummary,
        ContactVerifyError,
    > {
        let who = teller(MAIN);
        let service = ContactVerify::new(self.branch_tx());
        let mut tx = self.pool.begin().await.expect("begin");
        let answer = service
            .verify_in(&mut tx, &who, &BranchId::new(MAIN), customer_id, phone, now)
            .await;
        if answer.is_ok() {
            tx.commit().await.expect("commit");
        }
        answer
    }
}

fn teller(branch: &str) -> User {
    user(Uuid::new_v4(), Role::Teller, branch)
}

fn user(id: Uuid, role: Role, branch: &str) -> User {
    User::new(
        &id.to_string(),
        &xc_org().to_string(),
        vec![ScopedRole::new(
            role,
            Some(BranchId::new(branch)),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

fn mark(customer_id: &str, verified_at: DateTime<Utc>) -> MemberMark {
    MemberMark {
        customer_id: customer_id.to_owned(),
        verified_at: Some(verified_at),
    }
}

/// No form of the test phone appears in `text`.
fn assert_no_phone(text: &str, what: &str) {
    for form in [TYPED, E164, NATIONAL, SUBSCRIBER, &hex(&e164_hash())] {
        assert!(
            !text.contains(form),
            "{what} must never hold the phone ({form}): {text}"
        );
    }
}

// ── HTTP: the whole route table, callers named by a header ──────────────────

/// `x-test-as: <uuid>|<ROLE>@<BRANCH>`; the users are of `xc_org()`.
struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (id, rest) = spec.split_once('|')?;
    let (role, branch) = rest.split_once('@')?;
    let role = match role {
        "TELLER" => Role::Teller,
        "BRANCH_MANAGER" => Role::BranchManager,
        "FINANCE" => Role::Finance,
        "MARKETING" => Role::Marketing,
        _ => return None,
    };
    Some(user(Uuid::parse_str(id).ok()?, role, branch))
}

fn header_spec(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = header_spec(req);
        Box::pin(async move {
            let u = user_of(&spec);
            Callers {
                owner: None,
                staff: None,
                user: u.clone(),
                staff_user: u,
            }
        })
    }
}

/// A user row of `xc_org()` for `role` at `branch`; answers the `x-test-as` value.
async fn employ(db: &Db, role: &str, branch: &str) -> String {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2, $3, 'ACTIVE', 'test')",
    )
    .bind(id)
    .bind(xc_org())
    .bind(unique(role))
    .execute(&db.pool)
    .await
    .expect("insert a user");
    format!("{id}|{role}@{branch}")
}

/// The server's whole route table over `db`, callers named by `x-test-as`.
macro_rules! app {
    ($db:expr) => {{
        let state = AppState::new(Arc::new(common::settings(&[])), $db.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await
    }};
}

struct Answer {
    status: StatusCode,
    text: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text)
            .unwrap_or_else(|e| panic!("a JSON body ({e}): {}", self.text))
    }
}

async fn send<S, R, B>(app: &S, req: R) -> Answer
where
    S: Service<R, Response = ServiceResponse<B>, Error = actix_web::Error>,
    B: MessageBody,
{
    let resp = atest::call_service(app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn verify_call(
    as_: &str,
    key: &str,
    branch: &str,
    customer_id: &str,
    phone: &str,
) -> atest::TestRequest {
    atest::TestRequest::post()
        .uri(VERIFY)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("idempotency-key", key.to_owned()))
        .insert_header((AS, as_.to_owned()))
        .set_json(json!({ "branch": branch, "customerId": customer_id, "phone": phone }))
}

fn customers_call(as_: &str, branch: &str) -> atest::TestRequest {
    atest::TestRequest::default()
        .method(Method::GET)
        .uri(&format!(
            "/api/v3/views/admin_customers?branch={branch}&limit=100"
        ))
        .insert_header((header::HOST, HOST))
        .insert_header((AS, as_.to_owned()))
}

/// `customer_id`'s `member` in an `admin_customers` answer of `branch`.
fn member_in(answer: &Answer, branch: &str, customer_id: &str) -> Value {
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "admin_customers of {branch}: {:.400}",
        answer.text
    );
    let body = answer.json();
    body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("a list in data: {body}"))
        .iter()
        .find(|r| r["id"] == json!(customer_id))
        .unwrap_or_else(|| panic!("{customer_id} in {branch}'s admin_customers: {body}"))["member"]
        .clone()
}

fn assert_refused(
    answer: &Answer,
    status: StatusCode,
    code: &str,
    field: Option<&str>,
    what: &str,
) {
    assert_eq!(answer.status, status, "{what}: {:.400}", answer.text);
    let body = answer.json();
    assert_eq!(body["error"]["code"], json!(code), "{what}: {body}");
    if let Some(field) = field {
        assert_eq!(body["error"]["field"], json!(field), "{what}: {body}");
    }
}

// ── §9.1: verify writes one PHONE identity and one audit row ────────────────

#[tokio::test]
async fn verifying_a_phone_writes_one_active_verified_phone_identity_sealed_and_hashed() {
    let db = Db::start().await;
    let customer = db.customer(xc_org()).await;

    let summary = db
        .verify(&customer, TYPED, at(0))
        .await
        .expect("verifying a new phone succeeds");

    let rows = db.phone_identities(&customer).await;
    assert_eq!(rows.len(), 1, "exactly one PHONE identity: {rows:?}");
    let row = &rows[0];
    assert_eq!(row.r#type, "PHONE");
    assert_eq!(row.status, "ACTIVE");
    assert_eq!(row.organization_id, xc_org());
    assert_eq!(
        row.verified_at,
        Some(at(0)),
        "verified_at is the verification's time"
    );
    assert_eq!(
        row.lookup_hash,
        e164_hash(),
        "lookup_hash of the E.164 form"
    );
    for form in [TYPED, E164, NATIONAL, SUBSCRIBER] {
        assert!(
            !String::from_utf8_lossy(&row.encrypted_value).contains(form),
            "encrypted_value holds the phone in clear ({form})"
        );
    }
    let opened = PiiCipher::new(test_keys())
        .decrypt(
            format!("{customer}PHONE").as_bytes(),
            &Sealed {
                key_id: row.key_id.clone(),
                bytes: row.encrypted_value.clone(),
            },
        )
        .expect("the identity opens with AAD \"{customer_id}PHONE\"");
    assert_eq!(
        opened,
        E164.as_bytes(),
        "the sealed value is the E.164 form"
    );

    assert_eq!(summary.customer_id, customer);
    assert_eq!(summary.identity_id, row.id);
    assert_eq!(summary.verified_at, at(0));
    assert!(
        summary.phone_masked.ends_with(LAST4),
        "the summary shows the last four digits: {}",
        summary.phone_masked
    );
    assert_no_phone(&summary.phone_masked, "the summary");
    db.finish().await;
}

#[tokio::test]
async fn verifying_a_phone_writes_exactly_one_audit_row_that_never_holds_the_phone() {
    let db = Db::start().await;
    let customer = db.customer(xc_org()).await;
    assert_eq!(db.audit_count().await, 0, "a fresh schema has no audit row");

    db.verify(&customer, TYPED, at(0))
        .await
        .expect("verifying a new phone succeeds");

    let rows = db.audit_text().await;
    assert_eq!(rows.len(), 1, "exactly one audit row: {rows:?}");
    assert!(
        rows[0].contains(&customer),
        "the audit row names the customer: {}",
        rows[0]
    );
    assert_no_phone(&rows[0], "the audit row");
    db.finish().await;
}

// ── (c) the same phone twice ────────────────────────────────────────────────

#[tokio::test]
async fn verifying_the_same_phone_twice_keeps_one_identity_and_its_first_verified_at() {
    let db = Db::start().await;
    let customer = db.customer(xc_org()).await;

    let first = db
        .verify(&customer, TYPED, at(0))
        .await
        .expect("the first verification succeeds");
    let second = db
        .verify(&customer, NATIONAL, at(600))
        .await
        .expect("verifying the same phone again succeeds");

    let rows = db.phone_identities(&customer).await;
    assert_eq!(rows.len(), 1, "still one PHONE identity: {rows:?}");
    assert_eq!(rows[0].status, "ACTIVE");
    assert_eq!(
        rows[0].verified_at,
        Some(at(0)),
        "the first verified_at is kept"
    );
    assert_eq!(second, first, "the second answer is the same summary");
    db.finish().await;
}

// ── (d) a phone of another customer ─────────────────────────────────────────

#[tokio::test]
async fn a_phone_active_for_another_customer_of_the_org_is_validation_failed_and_writes_nothing() {
    let db = Db::start().await;
    let holder = db.customer(xc_org()).await;
    let held = db.seed_phone(xc_org(), &holder, E164, None, "ACTIVE").await;
    let customer = db.customer(xc_org()).await;
    let before = db.phone_identities(&holder).await;

    match db.verify(&customer, TYPED, at(0)).await {
        Err(ContactVerifyError::ValidationFailed(message)) => {
            assert_no_phone(&message, "the refusal")
        }
        other => panic!("a phone ACTIVE for another customer is ValidationFailed: {other:?}"),
    }

    assert!(
        db.phone_identities(&customer).await.is_empty(),
        "no identity for the refused customer"
    );
    assert_eq!(
        db.phone_identities(&holder).await,
        before,
        "the holder's identity {held} is untouched"
    );
    assert_eq!(db.audit_count().await, 0, "no audit row for a refusal");
    db.finish().await;
}

#[tokio::test]
async fn the_same_phone_active_in_another_organisation_is_allowed() {
    let db = Db::start().await;
    let other = db.other_org().await;
    let elsewhere = db.customer(other).await;
    db.seed_phone(other, &elsewhere, E164, Some(at(-60)), "ACTIVE")
        .await;
    let customer = db.customer(xc_org()).await;

    db.verify(&customer, TYPED, at(0))
        .await
        .expect("a phone of another organisation's customer does not block this one");

    let rows = db.phone_identities(&customer).await;
    assert_eq!(rows.len(), 1, "one PHONE identity: {rows:?}");
    assert_eq!(rows[0].verified_at, Some(at(0)));
    assert_eq!(rows[0].organization_id, xc_org());
    db.finish().await;
}

// ── the member flag: admin_customers ────────────────────────────────────────

#[actix_web::test]
async fn admin_customers_flags_a_verified_phone_member_at_every_branch_and_nothing_else() {
    let db = Db::start().await;
    let app = app!(db);
    let at_main = employ(&db, "TELLER", MAIN).await;
    let at_b2 = employ(&db, "TELLER", B2).await;

    let verified = db.customer(xc_org()).await;
    db.seed_phone(xc_org(), &verified, E164, Some(at(-60)), "ACTIVE")
        .await;
    let no_phone = db.customer(xc_org()).await;
    let unverified = db.customer(xc_org()).await;
    db.seed_phone(xc_org(), &unverified, "+66811111111", None, "ACTIVE")
        .await;
    let revoked = db.customer(xc_org()).await;
    db.seed_phone(
        xc_org(),
        &revoked,
        "+66822222222",
        Some(at(-120)),
        "REVOKED",
    )
    .await;
    let v2_member = db.customer(xc_org()).await;
    db.seed_member(MAIN, &v2_member).await;

    for (as_, branch) in [(&at_main, MAIN), (&at_b2, B2)] {
        assert_eq!(
            member_in(
                &send(&app, customers_call(as_, branch).to_request()).await,
                branch,
                &verified,
            ),
            json!(true),
            "a verified PHONE identity is a member at {branch}"
        );
        for (who, what) in [
            (&no_phone, "no phone"),
            (&unverified, "an unverified phone"),
            (&revoked, "a REVOKED verified phone"),
            (&v2_member, "a v2 members row only"),
        ] {
            assert_eq!(
                member_in(
                    &send(&app, customers_call(as_, branch).to_request()).await,
                    branch,
                    who,
                ),
                json!(false),
                "{what} is not a member at {branch}"
            );
        }
    }
    db.finish().await;
}

// ── the member flag: the compat snapshot ────────────────────────────────────

#[tokio::test]
async fn the_snapshot_member_marks_come_from_the_verified_phone_identity_not_members() {
    let db = Db::start().await;
    let verified = db.customer(xc_org()).await;
    db.seed_phone(xc_org(), &verified, E164, Some(at(-60)), "ACTIVE")
        .await;
    let unverified = db.customer(xc_org()).await;
    db.seed_phone(xc_org(), &unverified, "+66811111111", None, "ACTIVE")
        .await;
    let revoked = db.customer(xc_org()).await;
    db.seed_phone(
        xc_org(),
        &revoked,
        "+66822222222",
        Some(at(-120)),
        "REVOKED",
    )
    .await;
    let v2_member = db.customer(xc_org()).await;
    db.seed_member(MAIN, &v2_member).await;
    let other = db.other_org().await;
    let elsewhere = db.customer(other).await;
    db.seed_phone(other, &elsewhere, "+66833333333", Some(at(-30)), "ACTIVE")
        .await;

    for branch in [MAIN, B2] {
        assert_eq!(
            db.snapshot_marks(branch).await,
            vec![mark(&verified, at(-60))],
            "{branch}: the one verified PHONE customer of the organisation, with its verified_at; \
             no mark for an unverified or REVOKED phone, a members row, or another organisation"
        );
    }
    db.finish().await;
}

// ── the member flag: Broker's BookingMemberKeys ─────────────────────────────

#[tokio::test]
async fn an_exchange_whose_blueprint_booking_customer_has_a_verified_phone_carries_that_phone() {
    let db = Db::start().await;
    let customer = db.customer(xc_org()).await;
    db.seed_phone(xc_org(), &customer, E164, Some(at(-60)), "ACTIVE")
        .await;
    let (exchange, _) = db.seed_transaction(&customer).await;
    db.seed_completed_booking(&customer, exchange).await;

    assert_eq!(
        db.keys_of(exchange).await,
        CustomerKeys {
            customer_sub: None,
            phone: Some(E164.to_owned()),
        },
        "the booking's customer's verified phone, opened"
    );
    db.finish().await;
}

#[tokio::test]
async fn no_blueprint_booking_or_no_verified_active_phone_gives_no_keys_and_no_error() {
    let db = Db::start().await;

    // no booking at all
    let walk_in = db.customer(xc_org()).await;
    db.seed_phone(xc_org(), &walk_in, "+66844444444", Some(at(-60)), "ACTIVE")
        .await;
    let (no_booking, _) = db.seed_transaction(&walk_in).await;

    // a booking whose customer's phone is not verified
    let unverified = db.customer(xc_org()).await;
    db.seed_phone(xc_org(), &unverified, "+66811111111", None, "ACTIVE")
        .await;
    let (unverified_ex, _) = db.seed_transaction(&unverified).await;
    db.seed_completed_booking(&unverified, unverified_ex).await;

    // a booking whose customer's verified phone is REVOKED
    let revoked = db.customer(xc_org()).await;
    db.seed_phone(
        xc_org(),
        &revoked,
        "+66822222222",
        Some(at(-120)),
        "REVOKED",
    )
    .await;
    let (revoked_ex, _) = db.seed_transaction(&revoked).await;
    db.seed_completed_booking(&revoked, revoked_ex).await;

    // a v2 member's v2 booking, the source BK-10 retires
    let v2_member = db.customer(xc_org()).await;
    db.seed_member(MAIN, &v2_member).await;
    let member_id = db.member_id_of(&v2_member).await;
    let (v2_ex, v2_ref) = db.seed_transaction(&v2_member).await;
    db.seed_v2_member_booking(&member_id, &v2_ref).await;

    for (exchange, what) in [
        (no_booking, "an exchange with no booking"),
        (
            unverified_ex,
            "a booking whose customer's phone is unverified",
        ),
        (
            revoked_ex,
            "a booking whose customer's verified phone is REVOKED",
        ),
        (v2_ex, "a v2 member's v2 booking"),
    ] {
        assert_eq!(
            db.keys_of(exchange).await,
            CustomerKeys::default(),
            "{what}: no keys"
        );
    }
    db.finish().await;
}

// ── HTTP: customerContactVerify ─────────────────────────────────────────────

#[actix_web::test]
async fn customer_contact_verify_admits_teller_and_branch_manager_and_refuses_roles_without_customer_hold_place()
 {
    let db = Db::start().await;
    let app = app!(db);

    for role in ["TELLER", "BRANCH_MANAGER"] {
        let as_ = employ(&db, role, MAIN).await;
        let customer = db.customer(xc_org()).await;
        let phone = if role == "TELLER" { TYPED } else { OTHER_TYPED };
        let answer = send(
            &app,
            verify_call(&as_, &unique("k"), MAIN, &customer, phone).to_request(),
        )
        .await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "{role} holds CustomerHoldPlace: {:.400}",
            answer.text
        );
        assert_eq!(answer.json()["customerId"], json!(customer), "{role}");
        assert_eq!(db.phone_identities(&customer).await.len(), 1, "{role}");
    }

    for role in ["FINANCE", "MARKETING"] {
        let as_ = employ(&db, role, MAIN).await;
        let customer = db.customer(xc_org()).await;
        let answer = send(
            &app,
            verify_call(&as_, &unique("k"), MAIN, &customer, "0877777777").to_request(),
        )
        .await;
        assert_refused(
            &answer,
            StatusCode::BAD_REQUEST,
            "roleNotAllowed",
            None,
            &format!("{role} lacks CustomerHoldPlace"),
        );
        assert!(db.phone_identities(&customer).await.is_empty(), "{role}");
    }
    db.finish().await;
}

#[actix_web::test]
async fn customer_contact_verify_answers_the_phone_masked_and_replays_by_idempotency_key() {
    let db = Db::start().await;
    let app = app!(db);
    let as_ = employ(&db, "TELLER", MAIN).await;
    let customer = db.customer(xc_org()).await;
    let key = unique("k");

    let first = send(
        &app,
        verify_call(&as_, &key, MAIN, &customer, TYPED).to_request(),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{:.400}", first.text);
    assert_no_phone(&first.text, "the answer");
    assert!(
        first.text.contains(LAST4),
        "the answer shows the last four digits: {}",
        first.text
    );
    let body = first.json();
    assert_eq!(body["customerId"], json!(customer), "{body}");

    let replay = send(
        &app,
        verify_call(&as_, &key, MAIN, &customer, TYPED).to_request(),
    )
    .await;
    assert_eq!(replay.status, StatusCode::OK, "{:.400}", replay.text);
    assert_eq!(
        replay.json(),
        body,
        "the same key and body replay the answer"
    );
    assert_eq!(
        db.phone_identities(&customer).await.len(),
        1,
        "written once"
    );
    assert_eq!(db.audit_count().await, 1, "audited once");

    let conflict = send(
        &app,
        verify_call(&as_, &key, MAIN, &customer, OTHER_TYPED).to_request(),
    )
    .await;
    assert_refused(
        &conflict,
        StatusCode::CONFLICT,
        "idempotencyConflict",
        None,
        "the same key with another body",
    );
    let rows = db.phone_identities(&customer).await;
    assert_eq!(rows.len(), 1, "the conflict writes nothing: {rows:?}");
    assert_eq!(rows[0].lookup_hash, e164_hash(), "still the first phone");
    db.finish().await;
}

#[actix_web::test]
async fn customer_contact_verify_refuses_another_orgs_customer_a_bad_phone_and_a_phone_in_use() {
    let db = Db::start().await;
    let app = app!(db);
    let as_ = employ(&db, "TELLER", MAIN).await;

    let other = db.other_org().await;
    let foreign = db.customer(other).await;
    let answer = send(
        &app,
        verify_call(&as_, &unique("k"), MAIN, &foreign, TYPED).to_request(),
    )
    .await;
    assert_refused(
        &answer,
        StatusCode::FORBIDDEN,
        "outOfScope",
        None,
        "a customer of another organisation",
    );
    assert!(db.phone_identities(&foreign).await.is_empty());

    let customer = db.customer(xc_org()).await;
    let answer = send(
        &app,
        verify_call(&as_, &unique("k"), MAIN, &customer, "not-a-phone").to_request(),
    )
    .await;
    assert_refused(
        &answer,
        StatusCode::BAD_REQUEST,
        "validationFailed",
        Some("phone"),
        "a bad phone",
    );

    let holder = db.customer(xc_org()).await;
    db.seed_phone(xc_org(), &holder, E164, Some(at(-60)), "ACTIVE")
        .await;
    let answer = send(
        &app,
        verify_call(&as_, &unique("k"), MAIN, &customer, TYPED).to_request(),
    )
    .await;
    assert_refused(
        &answer,
        StatusCode::BAD_REQUEST,
        "validationFailed",
        Some("phone"),
        "a phone ACTIVE for another customer of the organisation",
    );
    assert_no_phone(&answer.text, "the refusal");

    assert!(
        db.phone_identities(&customer).await.is_empty(),
        "no refusal writes an identity"
    );
    assert_eq!(db.audit_count().await, 0, "no refusal writes an audit row");
    db.finish().await;
}

#[actix_web::test]
async fn a_phone_verified_at_one_branch_flags_the_customer_member_at_another_branch() {
    let db = Db::start().await;
    let app = app!(db);
    let at_main = employ(&db, "TELLER", MAIN).await;
    let at_b2 = employ(&db, "TELLER", B2).await;
    let customer = db.customer(xc_org()).await;

    let answer = send(
        &app,
        verify_call(&at_main, &unique("k"), MAIN, &customer, TYPED).to_request(),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{:.400}", answer.text);

    assert_eq!(
        member_in(
            &send(&app, customers_call(&at_b2, B2).to_request()).await,
            B2,
            &customer,
        ),
        json!(true),
        "verified at MAIN, a member at B2"
    );
    assert_eq!(
        db.snapshot_marks(B2).await,
        vec![mark(
            &customer,
            db.phone_identities(&customer).await[0].verified_at.unwrap()
        )],
        "and in B2's snapshot"
    );
    db.finish().await;
}

// ── round 2, ruling 4 (a): a different number replaces the old phone ────────

#[tokio::test]
async fn verifying_a_different_number_revokes_the_old_phone_and_keeps_one_active_phone() {
    let db = Db::start().await;
    let customer = db.customer(xc_org()).await;
    let old = db
        .seed_phone(xc_org(), &customer, "+66899999999", Some(at(-60)), "ACTIVE")
        .await;

    let summary = db
        .verify(&customer, TYPED, at(0))
        .await
        .expect("verifying a new number succeeds");

    let rows = db.phone_identities(&customer).await;
    let active: Vec<&IdentityRow> = rows.iter().filter(|r| r.status == "ACTIVE").collect();
    assert_eq!(active.len(), 1, "exactly one ACTIVE PHONE: {rows:?}");
    assert_eq!(
        active[0].id, summary.identity_id,
        "the new number is the ACTIVE one"
    );
    assert_eq!(active[0].lookup_hash, e164_hash());
    assert_eq!(active[0].verified_at, Some(at(0)));
    let old_row = rows
        .iter()
        .find(|r| r.id == old)
        .unwrap_or_else(|| panic!("the old identity {old} is kept, revoked: {rows:?}"));
    assert_eq!(old_row.status, "REVOKED", "the old number is revoked");
    db.finish().await;
}

// ── round 2, ruling 4 (b): every branch's admin_customers version moves ─────

#[actix_web::test]
async fn a_verification_moves_the_admin_customers_version_of_every_branch_of_the_organisation() {
    let db = Db::start().await;
    let app = app!(db);
    db.other_org().await;
    let at_main = employ(&db, "TELLER", MAIN).await;
    let at_b2 = employ(&db, "TELLER", B2).await;
    let customer = db.customer(xc_org()).await;
    let versions = BranchVersions::new(db.pool.clone());
    let version = |b: &'static str| {
        let versions = versions.clone();
        async move {
            versions
                .current(&BranchId::new(b))
                .await
                .expect("read a branch version")
        }
    };
    let b2_before = version(B2).await;
    let other_before = version(OTHER_BRANCH).await;
    let since = format!("/api/v3/views/admin_customers?branch={B2}&limit=100&since={b2_before}");
    let unchanged = send(
        &app,
        atest::TestRequest::default()
            .method(Method::GET)
            .uri(&since)
            .insert_header((header::HOST, HOST))
            .insert_header((AS, at_b2.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(unchanged.status, StatusCode::OK, "{:.400}", unchanged.text);
    assert_eq!(
        unchanged.json()["changed"],
        json!(false),
        "before: {}",
        unchanged.text
    );

    let answer = send(
        &app,
        verify_call(&at_main, &unique("k"), MAIN, &customer, TYPED).to_request(),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{:.400}", answer.text);

    let b2_after = version(B2).await;
    assert!(
        b2_after > b2_before,
        "a verification at MAIN moves B2's version ({b2_before} → {b2_after})"
    );
    assert_eq!(
        version(OTHER_BRANCH).await,
        other_before,
        "another organisation's branch does not move"
    );
    let changed = send(
        &app,
        atest::TestRequest::default()
            .method(Method::GET)
            .uri(&since)
            .insert_header((header::HOST, HOST))
            .insert_header((AS, at_b2.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK, "{:.400}", changed.text);
    let body = changed.json();
    assert_eq!(
        body["changed"],
        json!(true),
        "B2's same `since` now: {body}"
    );
    let row = body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("a list in data: {body}"))
        .iter()
        .find(|r| r["id"] == json!(customer))
        .unwrap_or_else(|| panic!("{customer} in B2's page: {body}"))
        .clone();
    assert_eq!(row["member"], json!(true), "{row}");
    db.finish().await;
}

// ── round 3, ruling 2: the every-branch bump never deadlocks ────────────────

/// How many rounds of the concurrent pair run, each with its own customers and phones.
const CONCURRENT_ROUNDS: usize = 5;

/// Two verifications at two different branches of one organisation, sent at once through the
/// whole route table (the command seam runs each in `BranchTx::run` on its acting branch), both
/// succeed within 10 seconds: the every-branch version bump takes no branch lock out of the
/// MAIN-first order (`xc-kernel/src/branch_tx.rs` lock order). A lock cycle shows as a PostgreSQL
/// deadlock error (40P01, a refused answer) or as a hang (the timeout).
///
/// Overlap: before each round the test holds `customers` and `customer_identities` in ACCESS
/// EXCLUSIVE mode, waits until both requests are blocked on them (each already holding its acting
/// branch's lock), then lets both go at once. A verification that reads neither table before its
/// cross-branch bump is not held there; for it the overlap is only as likely as two requests
/// started together, which is why the pair runs several rounds.
#[actix_web::test]
async fn two_verifications_at_two_branches_at_once_both_succeed_without_a_lock_cycle() {
    let db = Db::start().await;
    let app = app!(db);
    let at_main = employ(&db, "TELLER", MAIN).await;
    let at_b2 = employ(&db, "TELLER", B2).await;
    let versions = BranchVersions::new(db.pool.clone());

    for round in 0..CONCURRENT_ROUNDS {
        let main_customer = db.customer(xc_org()).await;
        let b2_customer = db.customer(xc_org()).await;
        let main_phone = format!("08100000{round}1");
        let b2_phone = format!("08100000{round}2");
        let main_before = versions
            .current(&BranchId::new(MAIN))
            .await
            .expect("MAIN's version");
        let b2_before = versions
            .current(&BranchId::new(B2))
            .await
            .expect("B2's version");

        let mut gate = db.pool.begin().await.expect("begin the gate");
        sqlx::query("LOCK TABLE customers, customer_identities IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *gate)
            .await
            .expect("close the gate");
        let open_gate = async {
            let started = std::time::Instant::now();
            let mut held = 0_i64;
            while started.elapsed() < std::time::Duration::from_secs(3) {
                held = sqlx::query_scalar(
                    "SELECT count(DISTINCT pid) FROM pg_locks WHERE NOT granted \
                     AND relation IN ('customers'::regclass, 'customer_identities'::regclass)",
                )
                .fetch_one(&db.pool)
                .await
                .expect("read pg_locks");
                if held >= 2 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            gate.commit().await.expect("open the gate");
            held
        };

        let joined = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(
                send(
                    &app,
                    verify_call(&at_main, &unique("k"), MAIN, &main_customer, &main_phone)
                        .to_request(),
                ),
                send(
                    &app,
                    verify_call(&at_b2, &unique("k"), B2, &b2_customer, &b2_phone).to_request(),
                ),
                open_gate,
            )
        })
        .await;
        let (at_main_answer, at_b2_answer, held) = joined.unwrap_or_else(|_| {
            panic!("round {round}: two verifications at MAIN and B2 did not finish within 10 s (a lock cycle)")
        });
        println!("round {round}: {held} request(s) were held at the gate together");
        assert_eq!(
            at_main_answer.status,
            StatusCode::OK,
            "round {round}, the verification at MAIN: {:.400}",
            at_main_answer.text
        );
        assert_eq!(
            at_b2_answer.status,
            StatusCode::OK,
            "round {round}, the verification at B2: {:.400}",
            at_b2_answer.text
        );
        for customer in [&main_customer, &b2_customer] {
            let rows = db.phone_identities(customer).await;
            assert!(
                rows.len() == 1 && rows[0].status == "ACTIVE" && rows[0].verified_at.is_some(),
                "round {round}: {customer} has its verified phone: {rows:?}"
            );
        }
        for (branch, before) in [(MAIN, main_before), (B2, b2_before)] {
            let after = versions
                .current(&BranchId::new(branch))
                .await
                .expect("a branch version");
            assert!(
                after > before,
                "round {round}: {branch}'s version moved ({before} → {after})"
            );
        }
    }
    db.finish().await;
}
