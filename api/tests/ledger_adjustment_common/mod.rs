//! The harness of the FINANCE ledger adjustment tests (design `docs/design/blueprint/ledger-inventory.md` §13.9):
//! `ledger_adjustment_create_test.rs` (LA-5), `ledger_adjustment_post_test.rs` (LA-6) and
//! `ledger_adjustments_view_test.rs` (LA-7). Copied from `s1_cash_exception_review_test.rs`'s harness.
//!
//! 🔴 Written by the sub-controller claude/f32b85db before the implementation. Do not edit, delete or weaken.
//!
//! The §13.9 fixture: branch `XC-B01` holds a USD position `1000 / 32,500.00` (average 32.50) with SAFE USD 1000
//! and SAFE THB 10,000; `XC-B02` holds nothing. Callers are `access::Actor`s from the header `x-test-as`:
//! `ROLE@BRANCH#n`, `ROLE@*#n` for an organisation-wide row. Approved, pending and rejected adjustments are
//! written as rows directly, so the post and view tests need neither the create command nor a decision.

#![allow(dead_code)]

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

use super::common;

pub const MUST_FINISH: Duration = Duration::from_secs(30);
pub const AS: &str = "x-test-as";
pub const HOST: &str = "xc.test";
pub const SAME_ORIGIN: &str = "http://xc.test";
/// The organisation the migrations seed.
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
pub const B1: &str = "XC-B01";
pub const B2: &str = "XC-B02";

pub const CREATE: &str = "/api/v3/commands/ledger_adjustment_create";
pub const POST: &str = "/api/v3/commands/ledger_adjustment_post";
pub const CREATE_AUDIT: &str = "LEDGER_ADJUSTMENT_CREATE";
pub const POST_AUDIT: &str = "LEDGER_ADJUSTMENT_POST";

/// The §13.9 people.
pub const FIN_A: &str = "FINANCE@XC-B01#1";
pub const FIN_B: &str = "FINANCE@XC-B01#2";
pub const FIN_ORG: &str = "FINANCE@*#3";
pub const FIN_C: &str = "FINANCE@XC-B02#4";
pub const MGR: &str = "BRANCH_MANAGER@XC-B01#5";
pub const EXEC: &str = "EXECUTIVE@*#6";
pub const TELLER: &str = "TELLER@XC-B01#7";
pub const CALLERS: [&str; 7] = [FIN_A, FIN_B, FIN_ORG, FIN_C, MGR, EXEC, TELLER];

/// `files.key`s: one of each branch.
pub const FILE_B1: &str = "SLIP-B01";
pub const FILE_B2: &str = "SLIP-B02";

pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

pub fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{s:?} is a decimal: {e}"))
}

pub fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").expect("USD")
}

/// A fixed uuid per caller spec.
pub fn user_id(spec: &str) -> String {
    let (_, n) = spec.rsplit_once('#').expect("a spec ends in #n");
    format!(
        "00000000-0000-4000-8000-{:012}",
        n.parse::<u64>().expect("n")
    )
}

// ------------------------------------------------------------ database

pub struct Room {
    pub pool: PgPool,
    pub schema: String,
}

impl Room {
    /// A migrated schema with the §13.9 fixture.
    pub async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("la");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        let room = Room { pool, schema };
        for spec in CALLERS {
            sqlx::query(
                "INSERT INTO users (id, organization_id, name, status, created_by) \
                 VALUES ($1::uuid, $2::uuid, $3, 'ACTIVE', 'test')",
            )
            .bind(user_id(spec))
            .bind(ORG)
            .bind(spec)
            .execute(&room.pool)
            .await
            .expect("a user");
        }
        for branch in [B1, B2, "MAIN"] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, timezone) \
                 VALUES ($1, $1, $2::uuid, 'Asia/Bangkok') ON CONFLICT (id) DO NOTHING",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&room.pool)
            .await
            .expect("a branch");
            sqlx::query(
                "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
                 VALUES ($1, 1, $2::uuid, now()) ON CONFLICT (branch_id) DO NOTHING",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&room.pool)
            .await
            .expect("a branch version");
        }
        for (key, branch) in [(FILE_B1, B1), (FILE_B2, B2)] {
            sqlx::query(
                "INSERT INTO files (key, organization_id, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ($1, $2::uuid, $3, 'owner', 'application/pdf', 'sha', now())",
            )
            .bind(key)
            .bind(ORG)
            .bind(branch)
            .execute(&room.pool)
            .await
            .expect("a file");
        }
        // B01: buy USD 1000 for THB 32,500.00 into the safe, then THB 10,000 into the safe at face.
        let buy = SourceRef {
            source_type: SourceType::Exchange,
            source_id: unique("buy"),
            source_key: "seed".to_owned(),
        };
        room.post_seed(vec![
            leg(CashPlace::Customer, safe(B1), usd(), "1000", &buy),
            leg(
                CashPlace::Bank,
                CashPlace::Customer,
                CurrencyCode::THB,
                "32500.00",
                &buy,
            ),
        ])
        .await;
        let float = SourceRef {
            source_type: SourceType::Opening,
            source_id: unique("float"),
            source_key: "seed".to_owned(),
        };
        room.post_seed(vec![leg(
            CashPlace::Bank,
            safe(B1),
            CurrencyCode::THB,
            "10000.00",
            &float,
        )])
        .await;
        assert_eq!(
            room.position(B1, "USD").await,
            Some((d("1000"), d("32500.00")))
        );
        room
    }

    pub async fn post_seed(&self, reqs: Vec<PostingRequest>) {
        let mut tx = self.pool.begin().await.expect("begin");
        LedgerCashPostings::new()
            .post_in(&mut tx, &reqs, None)
            .await
            .expect("the seed posts");
        tx.commit().await.expect("commit");
    }

    pub async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    pub async fn audits(&self, action: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE action = $1")
            .bind(action)
            .fetch_one(&self.pool)
            .await
            .expect("count audit rows")
    }

    /// `(actor_id, object_ref, branch_id, after_redacted)` of every audit row of `action`, oldest first.
    pub async fn audit_rows(
        &self,
        action: &str,
    ) -> Vec<(String, String, Option<String>, Option<Value>)> {
        sqlx::query_as(
            "SELECT actor_id, object_ref, branch_id, after_redacted FROM audit_events WHERE action = $1 ORDER BY id",
        )
        .bind(action)
        .fetch_all(&self.pool)
        .await
        .expect("read audit rows")
    }

    /// `(quantity, cost_thb)` of `branch`'s position in `currency`.
    pub async fn position(&self, branch: &str, currency: &str) -> Option<(Decimal, Decimal)> {
        sqlx::query_as(
            "SELECT quantity, cost_thb FROM inventory_positions WHERE branch_id = $1 AND currency = $2",
        )
        .bind(branch)
        .bind(currency)
        .fetch_optional(&self.pool)
        .await
        .expect("read the position")
    }

    pub async fn version(&self, branch: &str) -> i64 {
        sqlx::query_scalar("SELECT version FROM branch_versions WHERE branch_id = $1")
            .bind(branch)
            .fetch_one(&self.pool)
            .await
            .expect("the branch version")
    }

    /// Ledger postings of adjustment `id`.
    pub async fn postings_of(&self, id: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM ledger_postings WHERE source_type = 'ADJUSTMENT' AND source_id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("count postings")
    }

    /// `(postings, entries, movements, lots, approval requests)` in the schema.
    pub async fn written(&self) -> (i64, i64, i64, i64, i64) {
        (
            self.count("SELECT count(*) FROM ledger_postings").await,
            self.count("SELECT count(*) FROM ledger_entries").await,
            self.count("SELECT count(*) FROM inventory_movements").await,
            self.count("SELECT count(*) FROM inventory_lots").await,
            self.count("SELECT count(*) FROM approval_requests").await,
        )
    }

    /// An adjustment of `branch` stored as its request in `status` (`PENDING`, `APPROVED`, `REJECTED`), requested
    /// by FIN_A; a decided one carries FIN_B's decision. Answers its id.
    pub async fn adjustment(&self, status: &str, payload: Value) -> String {
        let id = format!("ADJ-{}", Uuid::new_v4());
        let request: Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
               proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'LEDGER_ADJUSTMENT', $1, $2, 'h', '1', $3, $4, now(), $5, $6, $7::uuid) \
             RETURNING id",
        )
        .bind(B1)
        .bind(&id)
        .bind(&payload)
        .bind(user_id(FIN_A))
        .bind(status)
        .bind(format!("ledger-adjustment:{id}"))
        .bind(ORG)
        .fetch_one(&self.pool)
        .await
        .expect("an adjustment request");
        if status != "PENDING" {
            sqlx::query(
                "INSERT INTO approval_decisions (id, request_id, approver_id, approver_role, outcome, reason, at) \
                 VALUES (gen_random_uuid(), $1, $2, 'FINANCE', $3, 'checked the slip', now())",
            )
            .bind(request)
            .bind(user_id(FIN_B))
            .bind(status)
            .execute(&self.pool)
            .await
            .expect("its decision");
        }
        id
    }

    /// An active (`RESERVED`, expiring in an hour) reservation of `quantity` USD at XC-B01, on a SELL quote line
    /// of its own (rows written directly, as `ledger_available_check_test.rs` seeds the quote).
    pub async fn reserve_usd(&self, quantity: &str) {
        let (quote_id, pub_id, req_id, rate_line_id, line_id) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let user = user_id(MGR);
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) VALUES ($1, $2::uuid, 'RATE_PUBLICATION', $3, $4, 'test', '1', \
             '{}'::jsonb, $5, now(), 'APPROVED', $6)",
        )
        .bind(req_id)
        .bind(ORG)
        .bind(B1)
        .bind(pub_id.to_string())
        .bind(&user)
        .bind(format!("k-{req_id}"))
        .execute(&self.pool)
        .await
        .expect("insert approval request");
        sqlx::query(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, \
             draft_id, published_by) VALUES ($1, $2, \
             (SELECT COALESCE(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $2), now(), $3, \
             NULL, $4)",
        )
        .bind(pub_id)
        .bind(B1)
        .bind(req_id)
        .bind(&user)
        .execute(&self.pool)
        .await
        .expect("insert rate publication");
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, \
             unit_size, buy_rate, sell_rate, source_snapshot) \
             VALUES ($1, $2, 'USD', 'ALL', 1.0, 34.0, 35.0, '{}'::jsonb)",
        )
        .bind(rate_line_id)
        .bind(pub_id)
        .execute(&self.pool)
        .await
        .expect("insert published rate line");
        sqlx::query(
            "INSERT INTO quotes (id, organization_id, branch_id, type, channel, status, publication_id, \
             expires_at, created_by, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, 'INDICATIVE', 'DESK', 'ACTIVE', $4, now() + interval '1 hour', 'test', \
             now(), now())",
        )
        .bind(quote_id)
        .bind(ORG)
        .bind(B1)
        .bind(pub_id)
        .execute(&self.pool)
        .await
        .expect("insert quote");
        sqlx::query(
            "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, shop_side, \
             currency, denomination_group, quantity, unit_size, base_rate, final_rate, thb_amount, \
             fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, 'SELL', 'USD', 'ALL', $5::numeric, 1.0, 35.0, 35.0, 0, 0, 0, 1, \
             now(), now())",
        )
        .bind(line_id)
        .bind(ORG)
        .bind(quote_id)
        .bind(rate_line_id)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .expect("insert quote line");
        sqlx::query(
            "INSERT INTO stock_reservations (id, organization_id, branch_id, quote_line_id, currency, quantity, \
             expires_at, status) VALUES ($1, $2::uuid, $3, $4, 'USD', $5::numeric, now() + interval '1 hour', \
             'RESERVED')",
        )
        .bind(Uuid::new_v4())
        .bind(ORG)
        .bind(B1)
        .bind(line_id)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .expect("an active reservation");
    }

    pub async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

pub fn safe(branch: &str) -> CashPlace {
    CashPlace::Safe(BranchId::new(branch))
}

pub fn leg(
    from: CashPlace,
    to: CashPlace,
    currency: CurrencyCode,
    qty: &str,
    source: &SourceRef,
) -> PostingRequest {
    PostingRequest {
        from,
        to,
        currency,
        quantity: ForeignAmount::from_decimal(d(qty)),
        source: source.clone(),
        line: None,
        reverses: None,
    }
}

/// §13.4's QUANTITY payload for `branch`: SAFE, `lines` as `(currency, signed quantity)`.
pub fn quantity_payload(branch: &str, lines: &[(&str, &str)]) -> Value {
    json!({
        "branch": branch,
        "kind": "QUANTITY",
        "place": "SAFE",
        "lines": lines.iter().map(|(c, q)| json!({ "currency": c, "quantity": q })).collect::<Vec<_>>(),
        "reason": "Bank deposit keyed as 1,100 USD; the slip says 1,000",
        "evidence": { "files": [FILE_B1], "reference": "slip 004512" }
    })
}

// ------------------------------------------------------------ callers

/// `x-test-as: <ROLE>@<branch or *>#<n>`: one Blueprint role row, with user id `user_id(spec)`.
struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role, rest) = spec.split_once('@')?;
    let (branch, _) = rest.split_once('#').expect("a spec ends in #n");
    let role = match role {
        "FINANCE" => Role::Finance,
        "TELLER" => Role::Teller,
        "BRANCH_MANAGER" => Role::BranchManager,
        "EXECUTIVE" => Role::Executive,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &user_id(spec),
        ORG,
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

pub struct Answer {
    pub status: StatusCode,
    pub text: String,
    pub body: Value,
}

pub async fn call(
    room: &Room,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: &str,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
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
        .insert_header((AS, who));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

pub async fn create(room: &Room, who: &str, body: Value, key: &str) -> Answer {
    call(room, who, Method::POST, CREATE, Some(body), key).await
}

pub async fn post(room: &Room, who: &str, branch: &str, id: &str, key: &str) -> Answer {
    call(
        room,
        who,
        Method::POST,
        POST,
        Some(json!({ "branch": branch, "id": id })),
        key,
    )
    .await
}

pub fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

pub fn field(a: &Answer) -> String {
    a.body["error"]["field"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

pub fn assert_refused(a: &Answer, status: StatusCode, want: &str, what: &str) {
    assert_eq!(
        (a.status, code(a).as_str()),
        (status, want),
        "{what}: {}",
        a.text
    );
}
