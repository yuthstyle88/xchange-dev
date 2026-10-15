//! Specification and edge-case tests for MK-6: audience segments (#1404, design `marketing-campaigns.md` §4.1, §6, §7, §8, §9.1).
//!
//! Amended by the MK-6 value ticket's author (claude/c4d72c, 2026-10-15; lane controller's ruling: D-14 value =
//! SUM(transaction_lines.thb_amount)): the seed helpers write a realistic row (wide `amount` = USD units = THB / 35,
//! `principal` and the new `transaction_lines.thb_amount` = the THB given), so every THB figure below is unchanged,
//! and a test pins the THB of every line.
//!
//! Pin table (§9 of #1404, updated with lane controller rulings Q1-Q5):
//! - Row 1: A customer whose newest MARKETING consent is withdrawn on every channel -> not in snapshot (D-13).
//! - Row 2: A customer granted on one channel only -> included in snapshot.
//! - Row 3: Consent withdrawn after a snapshot -> not in the next snapshot; earlier snapshot unchanged (frozen).
//! - Row 4: Two snapshots of one definition -> independent frozen rows.
//! - Row 5: Metrics over lookback (default 12 months) count only COMPLETED LIVE exchanges; DEMO ignored; reversed exchange kept (Q4).
//! - Row 6: No mirror row -> tierCode null; tier condition matches nobody.
//! - Row 7: A mirror row for customer's verified phone -> tierCode = mirror's code (stays red until LY-4 lands).
//! - Row 8: Defining a definition whose name already exists -> new version; earlier version unchanged.
//! - Row 9: `segment_preview` of a snapshot -> masked names only; no identity value (passport, ID, phone).
//! - Row 10: A role without MarketingMembers on each of the three routes -> 400 roleNotAllowed.
//! - M3 Schema: tables created, unique constraint on (organization_id, name, version), and FK campaign_versions.segment_snapshot_id -> segment_snapshots(id) (Q3).
//! - Policy: `areas/segments.rs` pins `MarketingMembers` on `EntryPath::Rpc` for all 3 routes (Q1).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgConnectOptions;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::Permission;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_auth::domain::policy::areas::segments::ROWS as SEGMENT_POLICY_ROWS;
use xc_auth::domain::policy::entities::policy::Need;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiKeys};
use xc_loyalty::infrastructure::pg_membership_mirror::PgMembershipMirror;
use xc_marketing::application::segment_service::SegmentSnapshotService;
use xc_marketing::domain::engine::consent_gate::ConsentGate;
use xc_marketing::domain::segment::metrics_source::CustomerMetricsSource;
use xc_marketing::domain::segment::rule::SegmentRule;
use xchange::infrastructure::idempotency::store::IdempotencyKey;
use xchange::infrastructure::marketing::pg_customer_metrics::PgCustomerMetrics;
use xchange::presentation::http::v3::command::CommandAction;
use xchange::presentation::http::v3::segment_define::{SegmentDefineCommand, SegmentDefineInput};
use xchange::presentation::http::v3::segment_preview::SegmentPreviewView;
use xchange::presentation::http::v3::segment_snapshot::{
    SegmentSnapshotCommand, SegmentSnapshotInput,
};

const MUST_FINISH: Duration = Duration::from_secs(30);
const ORG: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{nanos}_{n}")
}

fn org() -> Uuid {
    Uuid::parse_str(ORG).expect("the seeded organisation id")
}

fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
}

fn marketing_actor(org_id: Uuid, branch: &BranchId) -> Actor {
    Actor::new(
        "00000000-0000-0000-0000-000000000002",
        &org_id.to_string(),
        vec![ScopedRole::new(
            Role::Marketing,
            None,
            at(2026, 1, 1, 0),
            None,
        )],
        None,
    )
    .with_branch(Some(branch.clone()))
}

fn teller_actor(org_id: Uuid, branch: &BranchId) -> Actor {
    Actor::new(
        "00000000-0000-0000-0000-000000000003",
        &org_id.to_string(),
        vec![ScopedRole::new(
            Role::Teller,
            Some(branch.clone()),
            at(2026, 1, 1, 0),
            None,
        )],
        None,
    )
    .with_branch(Some(branch.clone()))
}

/// Stand-in ConsentGate wrapping the test database consents table
struct DbConsentGate {
    pool: PgPool,
}

impl DbConsentGate {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ConsentGate for DbConsentGate {
    fn marketing_allowed(&self, customer: &str) -> bool {
        let pool = self.pool.clone();
        let customer = customer.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let granted: Option<bool> = sqlx::query_scalar(
                    "WITH latest_per_channel AS ( \
                         SELECT DISTINCT ON (COALESCE(channel, '')) granted \
                         FROM consents \
                         WHERE customer_id = $1 AND purpose = 'MARKETING' \
                         ORDER BY COALESCE(channel, ''), effective_at DESC, created_at DESC \
                     ) \
                     SELECT bool_or(granted) FROM latest_per_channel",
                )
                .bind(customer)
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();
                granted.unwrap_or(false)
            })
        })
    }
}

struct Room {
    pool: PgPool,
    schema: String,
    branch: BranchId,
    user: Uuid,
}

impl Room {
    async fn open() -> Self {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL naming a _test database must be set");
        assert!(
            url.contains("_test"),
            "safety: TEST_DATABASE_URL must name a database ending in _test, got {url}"
        );
        let options = PgConnectOptions::from_str(&url).expect("valid connection string");
        let schema = unique("mk6_test");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        let branch_name = unique("b").replace('_', "-");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&branch_name)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("seed branch");

        let user_id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, 'marketing_user', 'ACTIVE', 'mk6-test') RETURNING id::text",
        )
        .bind(ORG)
        .fetch_one(&pool)
        .await
        .expect("seed marketing user");

        Room {
            pool,
            schema,
            branch: BranchId(branch_name),
            user: Uuid::parse_str(&user_id).unwrap(),
        }
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop schema cascade");
        pool.close().await;
    }
}

async fn insert_customer(pool: &PgPool, _branch: &BranchId, id: &str, name: &str, passport: &str) {
    let customer_no = unique("CUST");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $3, $4, 'LIVE', now())",
    )
    .bind(id)
    .bind(ORG)
    .bind(&customer_no)
    .bind(name)
    .execute(pool)
    .await
    .expect("seed customer");

    let ident_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, created_at) \
         VALUES ($1, $2::uuid, $3, 'PASSPORT', $4, 'k1', $5, now())",
    )
    .bind(ident_id)
    .bind(ORG)
    .bind(id)
    .bind(passport.as_bytes())
    .bind(passport.as_bytes())
    .execute(pool)
    .await
    .expect("seed customer identity");
}

fn pii_keys() -> PiiKeys {
    PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").expect("valid pii keys")
}

fn pii_hasher() -> LookupHasher {
    LookupHasher::new(&pii_keys())
}

fn phone_lookup_hash(phone: &str) -> Vec<u8> {
    pii_hasher().hash(IdentityType::Phone, phone).to_vec()
}

async fn insert_customer_with_phone(
    pool: &PgPool,
    _branch: &BranchId,
    id: &str,
    name: &str,
    phone: &str,
) {
    let customer_no = unique("CUST");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $3, $4, 'LIVE', now())",
    )
    .bind(id)
    .bind(ORG)
    .bind(&customer_no)
    .bind(name)
    .execute(pool)
    .await
    .expect("seed customer");

    let ident_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, verified_at, status, created_at) \
         VALUES ($1, $2::uuid, $3, 'PHONE', '\\x00', 'test-1', $4, now(), 'ACTIVE', now())",
    )
    .bind(ident_id)
    .bind(ORG)
    .bind(id)
    .bind(phone_lookup_hash(phone))
    .execute(pool)
    .await
    .expect("seed customer phone identity");
}

async fn insert_customer_tier_mirror(
    pool: &PgPool,
    organization_id: Uuid,
    phone_lookup_hash: &[u8],
    tier_code: &str,
) {
    sqlx::query(
        "INSERT INTO customer_tier_mirror ( \
             organization_id, phone_lookup_hash, tier_code, tier_rule_id, \
             tier_rule_version, effective_at, effective_until, loyalty_seq, synced_at \
         ) VALUES ($1, $2, $3, $4, 1, now(), NULL, 1, now())",
    )
    .bind(organization_id)
    .bind(phone_lookup_hash)
    .bind(tier_code)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed customer_tier_mirror");
}

async fn insert_consent(
    pool: &PgPool,
    customer_id: &str,
    channel: Option<&str>,
    granted: bool,
    effective_at: DateTime<Utc>,
) {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, effective_at, evidence_ref) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', $4, 1, 'th', $5, $6, 'test-proof')",
    )
    .bind(id)
    .bind(ORG)
    .bind(customer_id)
    .bind(channel)
    .bind(granted)
    .bind(effective_at)
    .execute(pool)
    .await
    .expect("seed consent");
}

async fn insert_tx(
    pool: &PgPool,
    branch: &BranchId,
    customer_id: &str,
    customer_name: &str,
    amount: &str,
    recorded_at: DateTime<Utc>,
) -> String {
    insert_tx_with_mode(
        pool,
        branch,
        customer_id,
        customer_name,
        amount,
        "LIVE",
        recorded_at,
    )
    .await
}

async fn insert_tx_with_mode(
    pool: &PgPool,
    branch: &BranchId,
    customer_id: &str,
    customer_name: &str,
    amount: &str,
    mode: &str,
    recorded_at: DateTime<Utc>,
) -> String {
    let sub = unique("sub");
    let doc = unique("doc");
    let shift_id = if mode == "LIVE" {
        let sid = unique("shift");
        let opn_req = unique("opn");
        sqlx::query(
            "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                 SELECT organization_id, 'Teller', 'ACTIVE', 'test' FROM branches WHERE id = $2 \
                 RETURNING id) \
             INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             SELECT $1, $2, 'CLOSED', now(), 'owner', $3, u.id FROM u \
             ON CONFLICT DO NOTHING",
        )
        .bind(&sid)
        .bind(branch.as_str())
        .bind(&opn_req)
        .execute(pool)
        .await
        .expect("seed shift");
        Some(sid)
    } else {
        None
    };

    let tx_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
         customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, bank_fee, received, paid, \
         method, payment_ref, account, staff_id, booking_no, notes, value_date, paid_at, recorded_at, completed_at, recorded_by) \
         VALUES ($1, $2, $3, $4, $5, $6, 'hash', $7, $8, 'TH', 'BUY', 'USD', round($9::numeric / 35, 2), 35.0, \
         $9::numeric, 0.0, 0.0, $9::numeric, $9::numeric, 'CASH', 'ref', 'acc', 'staff', 'bkg', 'note', \
         CURRENT_DATE, $10, $10, $10, 'staff')",
    )
    .bind(tx_id)
    .bind(&doc)
    .bind(branch.as_str())
    .bind(mode)
    .bind(shift_id)
    .bind(sub)
    .bind(customer_id)
    .bind(customer_name)
    .bind(amount)
    .bind(recorded_at)
    .execute(pool)
    .await
    .expect("seed transaction");
    insert_line(pool, tx_id, 1, "USD", "35.0", amount).await;

    tx_id.to_string()
}

#[allow(clippy::too_many_arguments)]
async fn insert_tx_with_mode_and_status(
    pool: &PgPool,
    branch: &BranchId,
    customer_id: &str,
    customer_name: &str,
    amount: &str,
    mode: &str,
    status: &str,
    recorded_at: DateTime<Utc>,
) -> String {
    let sub = unique("sub");
    let shift_id = if mode == "LIVE" {
        let sid = unique("shift");
        let opn_req = unique("opn");
        sqlx::query(
            "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                 SELECT organization_id, 'Teller', 'ACTIVE', 'test' FROM branches WHERE id = $2 \
                 RETURNING id) \
             INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             SELECT $1, $2, 'CLOSED', now(), 'owner', $3, u.id FROM u \
             ON CONFLICT DO NOTHING",
        )
        .bind(&sid)
        .bind(branch.as_str())
        .bind(&opn_req)
        .execute(pool)
        .await
        .expect("seed shift");
        Some(sid)
    } else {
        None
    };

    let tx_id = Uuid::new_v4();
    let (doc_ref, completed_at, cancelled_at, cancel_reason) = if status == "COMPLETED" {
        (Some(unique("doc")), Some(recorded_at), None, None)
    } else if status == "CANCELLED" {
        (None, None, Some(recorded_at), Some("customer cancelled"))
    } else {
        (None, None, None, None)
    };

    sqlx::query(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
         customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, bank_fee, received, paid, \
         method, payment_ref, account, staff_id, booking_no, notes, value_date, paid_at, recorded_at, completed_at, \
         status, cancelled_at, cancel_reason, recorded_by) \
         VALUES ($1, $2, $3, $4, $5, $6, 'hash', $7, $8, 'TH', 'BUY', 'USD', round($9::numeric / 35, 2), 35.0, \
         $9::numeric, 0.0, 0.0, $9::numeric, $9::numeric, 'CASH', 'ref', 'acc', 'staff', 'bkg', 'note', \
         CURRENT_DATE, $10, $10, $11, $12, $13, $14, 'staff')",
    )
    .bind(tx_id)
    .bind(doc_ref)
    .bind(branch.as_str())
    .bind(mode)
    .bind(shift_id)
    .bind(&sub)
    .bind(customer_id)
    .bind(customer_name)
    .bind(amount)
    .bind(recorded_at)
    .bind(completed_at)
    .bind(status)
    .bind(cancelled_at)
    .bind(cancel_reason)
    .execute(pool)
    .await
    .expect("seed transaction with status");
    insert_line(pool, tx_id, 1, "USD", "35.0", amount).await;

    tx_id.to_string()
}

/// One `transaction_lines` row of `tx_id`: `thb` THB of `currency` at `rate` THB a unit (quantity = thb / rate).
/// The exchange's value is the line's THB (`thb_amount`); the wide `transactions.amount` holds foreign units
/// (design `marketing-campaigns.md:84`, D-14: a segment's value is THB). Added by the MK-6 value ticket's author.
async fn insert_line(
    pool: &PgPool,
    tx_id: Uuid,
    line_no: i32,
    currency: &str,
    rate: &str,
    thb: &str,
) {
    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, quantity, unit_size, \
             rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
         SELECT gen_random_uuid(), b.organization_id, t.id, 'BUY', $2, round($4::numeric / $3::numeric, 4), 1, \
                $3::numeric, $4::numeric, 0, $4::numeric, $5, now(), now() \
           FROM transactions t JOIN branches b ON b.id = t.branch_id WHERE t.id = $1",
    )
    .bind(tx_id)
    .bind(currency)
    .bind(rate)
    .bind(thb)
    .bind(line_no)
    .execute(pool)
    .await
    .expect("seed a transaction line");
}

async fn insert_posted_reversal(
    pool: &PgPool,
    branch: &BranchId,
    original_tx_id: &str,
    reversal_at: DateTime<Utc>,
) {
    let rev_id = Uuid::new_v4();
    let orig_uuid = Uuid::parse_str(original_tx_id).expect("valid tx uuid");
    let file_key = unique("rev_file");
    let app_req_id = Uuid::new_v4();
    let receipt_id = Uuid::new_v4();
    let posting_id = Uuid::new_v4();
    let rno = unique("RCP");
    let ik = unique("ik");

    sqlx::query(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ($1, $2, 'test-owner', 'text/plain', 'hash', now())",
    )
    .bind(&file_key)
    .bind(branch.as_str())
    .execute(pool)
    .await
    .expect("seed reversal file");

    sqlx::query(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES ($1, 'REVERSAL', $2, 'obj', 'v1', 'p1', '{}'::jsonb, 'staff', now(), 'APPROVED', $3)",
    )
    .bind(app_req_id)
    .bind(branch.as_str())
    .bind(&ik)
    .execute(pool)
    .await
    .expect("seed reversal approval request");

    sqlx::query(
        "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, document_ref, created_at, updated_at, kind) \
         VALUES ($1, $2::uuid, $3, $4, 2, now(), $5, now(), now(), 'REVERSAL')",
    )
    .bind(receipt_id)
    .bind(ORG)
    .bind(orig_uuid)
    .bind(&rno)
    .bind(&file_key)
    .execute(pool)
    .await
    .expect("seed reversal receipt");

    sqlx::query(
        "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
         VALUES ($1, $2::uuid, $3, 'REVERSAL', $4, 'reversal', 'POSTED', $5, 'staff')",
    )
    .bind(posting_id)
    .bind(ORG)
    .bind(branch.as_str())
    .bind(rev_id.to_string())
    .bind(reversal_at)
    .execute(pool)
    .await
    .expect("seed reversal ledger posting");

    sqlx::query(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, \
         reason, status, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
         requested_by, requested_at, posted_by, posted_at) \
         VALUES ($1, $2::uuid, $3, $4, 'Customer requested', 'POSTED', 1500.0, CURRENT_DATE, \
         $5, $6, $7, 'staff', $8, 'staff', $8)",
    )
    .bind(rev_id)
    .bind(ORG)
    .bind(branch.as_str())
    .bind(orig_uuid)
    .bind(app_req_id)
    .bind(posting_id)
    .bind(receipt_id)
    .bind(reversal_at)
    .execute(pool)
    .await
    .expect("seed posted reversal");
}

// -----------------------------------------------------------------------------
// §9 Row 1: Consent withdrawn on every channel -> not in snapshot (D-13)
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn consent_withdrawn_on_every_channel_excluded_from_snapshot() {
    let room = Room::open().await;
    let customer = unique("c_withdrawn");
    insert_customer(
        &room.pool,
        &room.branch,
        &customer,
        "Withdrawn User",
        "AB1234567",
    )
    .await;

    insert_consent(
        &room.pool,
        &customer,
        Some("SMS"),
        false,
        at(2026, 10, 1, 10),
    )
    .await;
    insert_consent(
        &room.pool,
        &customer,
        Some("EMAIL"),
        false,
        at(2026, 10, 2, 10),
    )
    .await;
    insert_consent(
        &room.pool,
        &customer,
        Some("PUSH"),
        false,
        at(2026, 10, 3, 10),
    )
    .await;

    insert_tx(
        &room.pool,
        &room.branch,
        &customer,
        "Withdrawn User",
        "5000.00",
        at(2026, 10, 8, 12),
    )
    .await;

    let gate = Arc::new(DbConsentGate::new(room.pool.clone()));
    let metrics_source = Arc::new(PgCustomerMetrics::new(room.pool.clone(), None));
    let service = SegmentSnapshotService::new(room.pool.clone(), metrics_source, gate);

    let rule = SegmentRule {
        tier: None,
        recency_days: Some(30),
        frequency: Some(1),
        value_thb: None,
        home_branch: None,
        lookback_months: Some(12),
    };

    let (def_id, _v) = service
        .define(org(), "WithdrawnTest", rule, room.user)
        .await
        .expect("define segment");

    let (snap_id, count) = service
        .snapshot(org(), def_id, room.user, at(2026, 10, 10, 10))
        .await
        .expect("snapshot segment");

    assert_eq!(
        count, 0,
        "customer with all channels withdrawn must NOT be in snapshot"
    );

    let member_row_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap_id)
    .bind(&customer)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(member_row_count, 0);

    room.finish().await;
}

// -----------------------------------------------------------------------------
// §9 Row 2: Customer granted on one channel only -> included
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn consent_granted_on_one_channel_included_in_snapshot() {
    let room = Room::open().await;
    let c_email_only = unique("c_email");
    let c_sms_only = unique("c_sms");

    insert_customer(
        &room.pool,
        &room.branch,
        &c_email_only,
        "Email Only",
        "P1111111",
    )
    .await;
    insert_customer(
        &room.pool,
        &room.branch,
        &c_sms_only,
        "SMS Only",
        "P2222222",
    )
    .await;

    insert_consent(
        &room.pool,
        &c_email_only,
        Some("SMS"),
        false,
        at(2026, 10, 1, 10),
    )
    .await;
    insert_consent(
        &room.pool,
        &c_email_only,
        Some("EMAIL"),
        true,
        at(2026, 10, 2, 10),
    )
    .await;

    insert_consent(
        &room.pool,
        &c_sms_only,
        Some("SMS"),
        true,
        at(2026, 10, 1, 10),
    )
    .await;
    insert_consent(
        &room.pool,
        &c_sms_only,
        Some("PUSH"),
        false,
        at(2026, 10, 2, 10),
    )
    .await;

    insert_tx(
        &room.pool,
        &room.branch,
        &c_email_only,
        "Email Only",
        "2000.00",
        at(2026, 10, 5, 10),
    )
    .await;
    insert_tx(
        &room.pool,
        &room.branch,
        &c_sms_only,
        "SMS Only",
        "2000.00",
        at(2026, 10, 6, 10),
    )
    .await;

    let gate = Arc::new(DbConsentGate::new(room.pool.clone()));
    let metrics_source = Arc::new(PgCustomerMetrics::new(room.pool.clone(), None));
    let service = SegmentSnapshotService::new(room.pool.clone(), metrics_source, gate);

    let rule = SegmentRule {
        tier: None,
        recency_days: Some(30),
        frequency: Some(1),
        value_thb: None,
        home_branch: None,
        lookback_months: Some(12),
    };

    let (def_id, _v) = service
        .define(org(), "OneChannelTest", rule, room.user)
        .await
        .expect("define segment");

    let (snap_id, count) = service
        .snapshot(org(), def_id, room.user, at(2026, 10, 10, 10))
        .await
        .expect("snapshot segment");

    assert_eq!(
        count, 2,
        "both customers with at least one granted channel must be in snapshot"
    );

    let c1_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap_id)
    .bind(&c_email_only)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(c1_count, 1, "c_email_only must be recorded as member");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// §9 Row 3: Consent withdrawn after a snapshot -> not in next; earlier frozen
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn consent_withdrawn_after_snapshot_excluded_from_next_earlier_frozen() {
    let room = Room::open().await;
    let c = unique("c_withdraw_later");
    insert_customer(&room.pool, &room.branch, &c, "Withdraw Later", "W123456").await;
    insert_consent(&room.pool, &c, Some("SMS"), true, at(2026, 10, 1, 10)).await;
    insert_tx(
        &room.pool,
        &room.branch,
        &c,
        "Withdraw Later",
        "2500.00",
        at(2026, 10, 2, 10),
    )
    .await;

    let gate = Arc::new(DbConsentGate::new(room.pool.clone()));
    let metrics_source = Arc::new(PgCustomerMetrics::new(room.pool.clone(), None));
    let service = SegmentSnapshotService::new(room.pool.clone(), metrics_source, gate);

    let rule = SegmentRule {
        tier: None,
        recency_days: Some(30),
        frequency: Some(1),
        value_thb: None,
        home_branch: None,
        lookback_months: Some(12),
    };

    let (def_id, _) = service
        .define(org(), "WithdrawLaterTest", rule, room.user)
        .await
        .expect("define");

    // Snapshot 1: customer has granted consent -> included
    let (snap1_id, count1) = service
        .snapshot(org(), def_id, room.user, at(2026, 10, 3, 10))
        .await
        .expect("snapshot 1");
    assert_eq!(count1, 1, "snapshot 1 includes customer");

    // Customer withdraws consent on Oct 4
    insert_consent(&room.pool, &c, Some("SMS"), false, at(2026, 10, 4, 10)).await;

    // Snapshot 2: customer has withdrawn consent -> excluded
    let (snap2_id, count2) = service
        .snapshot(org(), def_id, room.user, at(2026, 10, 5, 10))
        .await
        .expect("snapshot 2");
    assert_eq!(count2, 0, "snapshot 2 excludes customer after withdrawal");

    // Earlier snapshot 1 is unchanged (frozen)
    let snap1_c_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap1_id)
    .bind(&c)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(
        snap1_c_count, 1,
        "earlier snapshot 1 remains frozen with customer"
    );

    let snap2_c_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap2_id)
    .bind(&c)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(snap2_c_count, 0, "snapshot 2 does not have customer");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// §9 Row 4: Two snapshots of one definition -> independent frozen rows
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn two_snapshots_of_one_definition_are_independent_frozen_rows() {
    let room = Room::open().await;
    let c1 = unique("c_s1");
    let c2 = unique("c_s2");

    insert_customer(&room.pool, &room.branch, &c1, "Customer 1", "P001").await;
    insert_customer(&room.pool, &room.branch, &c2, "Customer 2", "P002").await;

    insert_consent(&room.pool, &c1, Some("SMS"), true, at(2026, 10, 1, 10)).await;
    insert_consent(&room.pool, &c2, Some("SMS"), true, at(2026, 10, 1, 10)).await;

    // c1 transacts on Oct 2
    insert_tx(
        &room.pool,
        &room.branch,
        &c1,
        "Customer 1",
        "3000.00",
        at(2026, 10, 2, 10),
    )
    .await;

    let gate = Arc::new(DbConsentGate::new(room.pool.clone()));
    let metrics_source = Arc::new(PgCustomerMetrics::new(room.pool.clone(), None));
    let service = SegmentSnapshotService::new(room.pool.clone(), metrics_source, gate);

    let rule = SegmentRule {
        tier: None,
        recency_days: Some(30),
        frequency: Some(1),
        value_thb: None,
        home_branch: None,
        lookback_months: Some(12),
    };

    let (def_id, _) = service
        .define(org(), "TwoSnaps", rule, room.user)
        .await
        .expect("define");

    // Snapshot 1 as of Oct 3: c1 has transacted, c2 has not
    let (snap1_id, count1) = service
        .snapshot(org(), def_id, room.user, at(2026, 10, 3, 10))
        .await
        .expect("snapshot 1");
    assert_eq!(count1, 1, "snapshot 1 has 1 member (c1)");

    // c2 transacts on Oct 4
    insert_tx(
        &room.pool,
        &room.branch,
        &c2,
        "Customer 2",
        "4000.00",
        at(2026, 10, 4, 10),
    )
    .await;

    // Snapshot 2 as of Oct 5: both c1 and c2 have transacted
    let (snap2_id, count2) = service
        .snapshot(org(), def_id, room.user, at(2026, 10, 5, 10))
        .await
        .expect("snapshot 2");
    assert_eq!(count2, 2, "snapshot 2 has 2 members (c1 and c2)");

    assert_ne!(snap1_id, snap2_id, "snapshot ids must be distinct");

    // Verify snapshot 1 still has c1 only
    let snap1_c1: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap1_id)
    .bind(&c1)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(snap1_c1, 1, "snapshot 1 has c1");

    let snap1_c2: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap1_id)
    .bind(&c2)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(snap1_c2, 0, "snapshot 1 does NOT have c2 (frozen)");

    // Verify snapshot 2 has c2
    let snap2_c2: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap2_id)
    .bind(&c2)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(snap2_c2, 1, "snapshot 2 has c2");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// §9 Row 5: Metrics over lookback: COMPLETED LIVE only, DEMO ignored, reversal kept (Q4)
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn metrics_count_completed_live_lookback_only_demo_ignored_reversal_kept() {
    let room = Room::open().await;
    let c_qualifying = unique("c_qual");
    let c_demo_only = unique("c_demo");
    let c_expired_lookback = unique("c_exp");
    let c_non_completed = unique("c_non_comp");

    insert_customer(&room.pool, &room.branch, &c_qualifying, "Qual", "Q1").await;
    insert_customer(&room.pool, &room.branch, &c_demo_only, "Demo Only", "D1").await;
    insert_customer(&room.pool, &room.branch, &c_expired_lookback, "Exp", "E1").await;
    insert_customer(
        &room.pool,
        &room.branch,
        &c_non_completed,
        "Non Completed",
        "NC1",
    )
    .await;

    insert_consent(
        &room.pool,
        &c_qualifying,
        Some("SMS"),
        true,
        at(2026, 10, 1, 10),
    )
    .await;
    insert_consent(
        &room.pool,
        &c_demo_only,
        Some("SMS"),
        true,
        at(2026, 10, 1, 10),
    )
    .await;
    insert_consent(
        &room.pool,
        &c_expired_lookback,
        Some("SMS"),
        true,
        at(2026, 10, 1, 10),
    )
    .await;
    insert_consent(
        &room.pool,
        &c_non_completed,
        Some("SMS"),
        true,
        at(2026, 10, 1, 10),
    )
    .await;

    // c_qualifying:
    // 1 COMPLETED LIVE exchange (1000.00 THB) on Oct 1
    insert_tx(
        &room.pool,
        &room.branch,
        &c_qualifying,
        "Qual",
        "1000.00",
        at(2026, 10, 1, 10),
    )
    .await;

    // 1 COMPLETED LIVE exchange (1500.00 THB) on Oct 2 with a POSTED reversal -> STILL counts (Q4)
    let rev_tx_id = insert_tx(
        &room.pool,
        &room.branch,
        &c_qualifying,
        "Qual",
        "1500.00",
        at(2026, 10, 2, 10),
    )
    .await;
    insert_posted_reversal(&room.pool, &room.branch, &rev_tx_id, at(2026, 10, 3, 10)).await;

    // c_demo_only: 2 COMPLETED DEMO exchanges totaling 3000 THB (>=2 and >=2000 THB) -> DEMO never counts (Q4)
    // If mutant drops mode check, frequency 2 and value 3000 turns count 1 -> 2.
    insert_tx_with_mode(
        &room.pool,
        &room.branch,
        &c_demo_only,
        "Demo Only",
        "1500.00",
        "DEMO",
        at(2026, 10, 2, 10),
    )
    .await;
    insert_tx_with_mode(
        &room.pool,
        &room.branch,
        &c_demo_only,
        "Demo Only",
        "1500.00",
        "DEMO",
        at(2026, 10, 2, 11),
    )
    .await;

    // c_expired_lookback: 2 LIVE exchanges totaling 3000 THB, but 14 months ago (> 12 months lookback)
    // If mutant drops lookback filter, frequency 2 and value 3000 turns count 1 -> 2.
    insert_tx(
        &room.pool,
        &room.branch,
        &c_expired_lookback,
        "Exp",
        "1500.00",
        at(2025, 8, 1, 10),
    )
    .await;
    insert_tx(
        &room.pool,
        &room.branch,
        &c_expired_lookback,
        "Exp",
        "1500.00",
        at(2025, 8, 1, 11),
    )
    .await;

    // c_non_completed: 2 LIVE exchanges totaling 3000 THB inside lookback, but CANCELLED (not COMPLETED)
    // If mutant drops status = 'COMPLETED' check, frequency 2 and value 3000 turns count 1 -> 2.
    insert_tx_with_mode_and_status(
        &room.pool,
        &room.branch,
        &c_non_completed,
        "Non Completed",
        "1500.00",
        "LIVE",
        "CANCELLED",
        at(2026, 10, 2, 10),
    )
    .await;
    insert_tx_with_mode_and_status(
        &room.pool,
        &room.branch,
        &c_non_completed,
        "Non Completed",
        "1500.00",
        "LIVE",
        "CANCELLED",
        at(2026, 10, 2, 11),
    )
    .await;

    let gate = Arc::new(DbConsentGate::new(room.pool.clone()));
    let metrics_source = Arc::new(PgCustomerMetrics::new(room.pool.clone(), None));
    let service = SegmentSnapshotService::new(room.pool.clone(), metrics_source, gate);

    // Rule: frequency >= 2, value >= 2000 THB, lookback 12 months (recency_days dropped per review item 1)
    let rule = SegmentRule {
        tier: None,
        recency_days: None,
        frequency: Some(2),
        value_thb: Some(Decimal::from_str("2000.00").unwrap()),
        home_branch: None,
        lookback_months: Some(12),
    };

    let (def_id, _) = service
        .define(org(), "LookbackRuleTest", rule, room.user)
        .await
        .expect("define");

    let (snap_id, count) = service
        .snapshot(org(), def_id, room.user, at(2026, 10, 10, 10))
        .await
        .expect("snapshot");

    assert_eq!(
        count, 1,
        "only c_qualifying matches: 2 LIVE exchanges (including reversed) inside lookback; DEMO, expired, and non-completed ignored"
    );

    let qual_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap_id)
    .bind(&c_qualifying)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(qual_count, 1, "c_qualifying is a member");

    let demo_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap_id)
    .bind(&c_demo_only)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(demo_count, 0, "DEMO customer must NEVER be included");

    let exp_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap_id)
    .bind(&c_expired_lookback)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(
        exp_count, 0,
        "expired lookback customer must NEVER be included"
    );

    let non_comp_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap_id)
    .bind(&c_non_completed)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(
        non_comp_count, 0,
        "non-completed customer must NEVER be included"
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// D-14 (marketing-campaigns.md:84, :216): a segment's value is THB, the sum of every line's `thb_amount`
// -----------------------------------------------------------------------------

/// The value is the THB of everything the customer exchanged in the lookback, over every line of every exchange,
/// never foreign units: USD 100 (3,500 THB) and JPY 10,000 (2,400 THB) in one exchange, and USD worth 1,000 THB in a
/// second, is 6,900 THB. Before this fix it summed `transactions.amount`, line 1's foreign units only.
#[tokio::test]
async fn value_thb_is_the_thb_of_every_line_not_foreign_units() {
    let room = Room::open().await;
    let c = unique("c_value");
    insert_customer(&room.pool, &room.branch, &c, "Value", "V1").await;
    let two_lines = insert_tx(
        &room.pool,
        &room.branch,
        &c,
        "Value",
        "3500.00",
        at(2026, 10, 1, 10),
    )
    .await;
    insert_line(
        &room.pool,
        Uuid::parse_str(&two_lines).expect("a transaction id"),
        2,
        "JPY",
        "0.24",
        "2400.00",
    )
    .await;
    insert_tx(
        &room.pool,
        &room.branch,
        &c,
        "Value",
        "1000.00",
        at(2026, 10, 2, 10),
    )
    .await;

    let metrics = PgCustomerMetrics::new(room.pool.clone(), None)
        .load_candidate_metrics(org(), 12, at(2026, 10, 10, 10))
        .await
        .expect("load the metrics");
    let mine = metrics
        .iter()
        .find(|m| m.customer_id == c)
        .expect("the customer is a candidate");
    assert_eq!(
        mine.value_thb,
        Decimal::from_str("6900.00").unwrap(),
        "D-14: THB over every line (3,500 + 2,400 + 1,000), not foreign units"
    );
    assert_eq!(
        mine.frequency, 2,
        "two exchanges, whatever their line count"
    );
    room.finish().await;
}

// -----------------------------------------------------------------------------
// §9 Row 6: No tier row in mirror -> tierCode null; tier condition never matches
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn no_mirror_row_yields_null_tier_code_and_tier_condition_never_matches() {
    let room = Room::open().await;
    let c_notier = unique("c_notier");
    insert_customer(
        &room.pool,
        &room.branch,
        &c_notier,
        "No Tier User",
        "T123456",
    )
    .await;
    insert_consent(
        &room.pool,
        &c_notier,
        Some("SMS"),
        true,
        at(2026, 10, 1, 10),
    )
    .await;
    insert_tx(
        &room.pool,
        &room.branch,
        &c_notier,
        "No Tier User",
        "10000.00",
        at(2026, 10, 5, 10),
    )
    .await;

    let c_tiered = unique("c_tiered");
    let phone = "+66812345678";
    insert_customer_with_phone(&room.pool, &room.branch, &c_tiered, "Gold User", phone).await;
    insert_customer_tier_mirror(&room.pool, org(), &phone_lookup_hash(phone), "GOLD").await;
    insert_consent(
        &room.pool,
        &c_tiered,
        Some("SMS"),
        true,
        at(2026, 10, 1, 10),
    )
    .await;
    insert_tx(
        &room.pool,
        &room.branch,
        &c_tiered,
        "Gold User",
        "20000.00",
        at(2026, 10, 5, 10),
    )
    .await;

    let loyalty_mirror = Arc::new(PgMembershipMirror::new(room.pool.clone(), pii_hasher()));
    let metrics_source = Arc::new(PgCustomerMetrics::new(
        room.pool.clone(),
        Some(loyalty_mirror),
    ));

    let metrics = metrics_source
        .load_candidate_metrics(org(), 12, at(2026, 10, 10, 10))
        .await
        .expect("load metrics");

    let no_tier_metric = metrics.iter().find(|m| m.customer_id == c_notier);
    assert!(no_tier_metric.is_some(), "metrics found for customer");
    assert_eq!(
        no_tier_metric.unwrap().tier_code,
        None,
        "tierCode is null when no tier in mirror"
    );

    let tiered_metric = metrics.iter().find(|m| m.customer_id == c_tiered);
    assert!(tiered_metric.is_some(), "metrics found for tiered customer");
    assert_eq!(
        tiered_metric.unwrap().tier_code,
        Some("GOLD".to_string()),
        "tierCode is GOLD for customer with mirror row (stays red until LY-4 lands)"
    );

    let gate = Arc::new(DbConsentGate::new(room.pool.clone()));
    let service = SegmentSnapshotService::new(room.pool.clone(), metrics_source, gate);

    // Rule with tier condition requires "GOLD"
    let gold_rule = SegmentRule {
        tier: Some("GOLD".to_string()),
        recency_days: None,
        frequency: None,
        value_thb: None,
        home_branch: None,
        lookback_months: Some(12),
    };

    let (gold_def, _) = service
        .define(org(), "GoldMembers", gold_rule, room.user)
        .await
        .expect("define gold");
    let (snap_id, count) = service
        .snapshot(org(), gold_def, room.user, at(2026, 10, 10, 10))
        .await
        .expect("snapshot gold");
    assert_eq!(
        count, 1,
        "only customer with matching GOLD tier in mirror matches; customer with no tier row is excluded"
    );

    let no_tier_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(snap_id)
    .bind(&c_notier)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(
        no_tier_count, 0,
        "customer without mirror row never in snapshot"
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// §9 Row 7: A mirror row for customer's verified phone -> tierCode = mirror's code
// (Stays red by result until LY-4 lands, as customer_tier_mirror comes with LY-4 M1)
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn mirror_row_for_verified_phone_yields_tier_code() {
    let room = Room::open().await;
    let c = unique("c_tiered");
    let phone = "+66812345678";

    insert_customer_with_phone(&room.pool, &room.branch, &c, "Gold User", phone).await;
    insert_customer_tier_mirror(&room.pool, org(), &phone_lookup_hash(phone), "GOLD").await;
    insert_consent(&room.pool, &c, Some("SMS"), true, at(2026, 10, 1, 10)).await;
    insert_tx(
        &room.pool,
        &room.branch,
        &c,
        "Gold User",
        "20000.00",
        at(2026, 10, 5, 10),
    )
    .await;

    let loyalty_mirror = Arc::new(PgMembershipMirror::new(room.pool.clone(), pii_hasher()));
    let metrics_source = Arc::new(PgCustomerMetrics::new(
        room.pool.clone(),
        Some(loyalty_mirror),
    ));

    let metrics = metrics_source
        .load_candidate_metrics(org(), 12, at(2026, 10, 10, 10))
        .await
        .expect("load metrics");

    let customer_metric = metrics.iter().find(|m| m.customer_id == c);
    assert!(customer_metric.is_some(), "metrics found for customer");

    // Pin §9 row 7: tierCode must equal the mirror's tier code
    assert_eq!(
        customer_metric.unwrap().tier_code,
        Some("GOLD".to_string()),
        "mirror row for customer's verified phone yields tierCode = mirror's code (red until LY-4 lands)"
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// §9 Row 8: Defining a definition whose name already exists -> new version; earlier unchanged
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn defining_existing_name_creates_new_version_earlier_unchanged() {
    let room = Room::open().await;

    let gate = Arc::new(DbConsentGate::new(room.pool.clone()));
    let metrics_source = Arc::new(PgCustomerMetrics::new(room.pool.clone(), None));
    let service = SegmentSnapshotService::new(room.pool.clone(), metrics_source, gate);

    let rule1 = SegmentRule {
        tier: None,
        recency_days: Some(30),
        frequency: Some(1),
        value_thb: None,
        home_branch: None,
        lookback_months: Some(12),
    };
    let (id1, v1) = service
        .define(org(), "VIP", rule1.clone(), room.user)
        .await
        .expect("define v1");
    assert_eq!(v1, 1, "first definition is version 1");

    let rule2 = SegmentRule {
        tier: Some("GOLD".to_string()),
        recency_days: Some(15),
        frequency: Some(5),
        value_thb: Some(Decimal::from_str("50000.00").unwrap()),
        home_branch: None,
        lookback_months: Some(12),
    };
    let (id2, v2) = service
        .define(org(), "VIP", rule2.clone(), room.user)
        .await
        .expect("define v2");
    assert_eq!(v2, 2, "second definition with same name is version 2");
    assert_ne!(id1, id2, "each version has distinct id");

    // Version 1 row is unchanged (immutable)
    let v1_rule_json: serde_json::Value = sqlx::query_scalar(
        "SELECT rule FROM segment_definitions WHERE id = $1 AND organization_id = $2",
    )
    .bind(id1)
    .bind(org())
    .fetch_one(&room.pool)
    .await
    .expect("fetch v1 rule");
    let fetched_rule1: SegmentRule =
        serde_json::from_value(v1_rule_json).expect("deserialize v1 rule");
    assert_eq!(fetched_rule1, rule1, "version 1 rule remains unchanged");

    room.finish().await;
}

// -----------------------------------------------------------------------------
// §9 Row 9: `segment_preview` of snapshot -> masked names only; no identity values
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn preview_of_snapshot_shows_masked_names_only_and_never_passport_or_id() {
    let room = Room::open().await;
    let c = unique("c_mask");
    let real_name = "Somchai Prasert";
    let passport = "AA9876543";
    let phone = "+66819876543";

    insert_customer(&room.pool, &room.branch, &c, real_name, passport).await;
    let ident_phone_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, verified_at, status, created_at) \
         VALUES ($1, $2::uuid, $3, 'PHONE', '\\x00', 'test-1', $4, now(), 'ACTIVE', now())",
    )
    .bind(ident_phone_id)
    .bind(ORG)
    .bind(&c)
    .bind(phone_lookup_hash(phone))
    .execute(&room.pool)
    .await
    .expect("seed customer phone identity");

    insert_consent(&room.pool, &c, Some("SMS"), true, at(2026, 10, 1, 10)).await;
    insert_tx(
        &room.pool,
        &room.branch,
        &c,
        real_name,
        "1500.00",
        at(2026, 10, 2, 10),
    )
    .await;

    let gate = Arc::new(DbConsentGate::new(room.pool.clone()));
    let metrics_source = Arc::new(PgCustomerMetrics::new(room.pool.clone(), None));
    let service = SegmentSnapshotService::new(room.pool.clone(), metrics_source, gate);

    let rule = SegmentRule {
        tier: None,
        recency_days: None,
        frequency: None,
        value_thb: None,
        home_branch: None,
        lookback_months: Some(12),
    };

    let (def_id, _) = service
        .define(org(), "PreviewTest", rule, room.user)
        .await
        .expect("define");
    let (snap_id, _) = service
        .snapshot(org(), def_id, room.user, at(2026, 10, 5, 10))
        .await
        .expect("snapshot");

    let preview = SegmentPreviewView::new(room.pool.clone());
    let who = marketing_actor(org(), &room.branch);
    let view_dto = preview
        .load(&room.branch, &who, snap_id)
        .await
        .expect("preview load");

    assert_eq!(view_dto.member_count, 1);
    assert_eq!(view_dto.sample.len(), 1);

    let sample_member = &view_dto.sample[0];
    assert_ne!(
        sample_member.masked_name, real_name,
        "preview must not show unmasked name"
    );
    assert!(
        sample_member.masked_name.contains("sert") || sample_member.masked_name.contains("****"),
        "preview masks name keeping only tail: '{}'",
        sample_member.masked_name
    );

    let json = serde_json::to_string(&view_dto).expect("serialize preview dto");
    assert!(
        !json.contains(passport),
        "preview JSON must NEVER contain passport number"
    );
    assert!(
        !json.contains("passport"),
        "preview JSON must not have passport field"
    );
    assert!(
        !json.contains("819876543"),
        "preview JSON must NEVER contain phone number"
    );
    assert!(
        !json.contains("phone"),
        "preview JSON must not have phone field"
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// §9 Row 10: Role without MarketingMembers on each of the 3 routes -> 400 roleNotAllowed
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn role_without_marketing_members_refused_with_400_role_not_allowed() {
    let room = Room::open().await;

    let m_actor = marketing_actor(org(), &room.branch);
    let t_actor = teller_actor(org(), &room.branch);

    let preview = SegmentPreviewView::new(room.pool.clone());
    let dummy_snap = Uuid::new_v4();

    // 1. View: segment_preview
    // Marketing role has MarketingMembers -> proceeds past gate
    let marketing_res = preview.load(&room.branch, &m_actor, dummy_snap).await;
    if let Err(pos_error::AppError::Validation(ref msg)) = marketing_res {
        assert_ne!(msg, "roleNotAllowed", "Marketing role must have permission");
    }

    // Teller role lacks MarketingMembers -> 400 roleNotAllowed
    let teller_res = preview.load(&room.branch, &t_actor, dummy_snap).await;
    match teller_res {
        Err(pos_error::AppError::Validation(ref msg)) => {
            assert_eq!(
                msg, "roleNotAllowed",
                "Teller actor must be refused with roleNotAllowed on view"
            );
        }
        other => panic!("expected roleNotAllowed refusal, got {other:?}"),
    }

    // 2. Command: segment_define
    let def_cmd = SegmentDefineCommand;
    let def_input = SegmentDefineInput {
        name: "GateTest".to_string(),
        rule: SegmentRule {
            tier: None,
            recency_days: None,
            frequency: None,
            value_thb: None,
            home_branch: None,
            lookback_months: None,
        },
    };
    let mut tx = room.pool.begin().await.expect("begin tx");
    let teller_def_res = def_cmd
        .run(
            &mut tx,
            &room.branch,
            &t_actor,
            def_input,
            IdempotencyKey::new(Uuid::new_v4()),
        )
        .await;
    match teller_def_res {
        Err(ref e) => {
            assert_eq!(
                e.code, "roleNotAllowed",
                "Teller refused on segmentDefine with roleNotAllowed"
            );
            assert_eq!(e.status, actix_web::http::StatusCode::BAD_REQUEST);
        }
        Ok(_) => panic!("expected roleNotAllowed refusal on segmentDefine for Teller"),
    }

    // 3. Command: segment_snapshot
    let snap_cmd = SegmentSnapshotCommand;
    let snap_input = SegmentSnapshotInput {
        id: Uuid::new_v4().to_string(),
    };
    let teller_snap_res = snap_cmd
        .run(
            &mut tx,
            &room.branch,
            &t_actor,
            snap_input,
            IdempotencyKey::new(Uuid::new_v4()),
        )
        .await;
    match teller_snap_res {
        Err(ref e) => {
            assert_eq!(
                e.code, "roleNotAllowed",
                "Teller refused on segmentSnapshot with roleNotAllowed"
            );
            assert_eq!(e.status, actix_web::http::StatusCode::BAD_REQUEST);
        }
        Ok(_) => panic!("expected roleNotAllowed refusal on segmentSnapshot for Teller"),
    }

    drop(tx);
    room.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn segment_define_command_succeeds_for_marketing_actor() {
    let room = Room::open().await;
    let m_actor = marketing_actor(org(), &room.branch);
    let def_cmd = SegmentDefineCommand;
    let def_input = SegmentDefineInput {
        name: "CommandVIP".to_string(),
        rule: SegmentRule {
            tier: None,
            recency_days: Some(30),
            frequency: Some(1),
            value_thb: Some(Decimal::from_str("5000.00").unwrap()),
            home_branch: None,
            lookback_months: Some(12),
        },
    };
    let mut tx = room.pool.begin().await.expect("begin tx");
    let out = def_cmd
        .run(
            &mut tx,
            &room.branch,
            &m_actor,
            def_input,
            IdempotencyKey::new(Uuid::new_v4()),
        )
        .await
        .expect("segmentDefine command succeeds for Marketing actor");
    tx.commit().await.expect("commit tx");

    assert_eq!(out.version, 1, "created definition at version 1");
    assert_ne!(out.id, Uuid::nil(), "returns non-nil definition id");

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_definitions WHERE id = $1 AND name = 'CommandVIP' AND version = 1",
    )
    .bind(out.id)
    .fetch_one(&room.pool)
    .await
    .expect("query definition");
    assert_eq!(count, 1, "definition row persisted");

    room.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn segment_snapshot_command_filters_withdrawn_and_counts_granted_for_marketing_actor() {
    let room = Room::open().await;
    let m_actor = marketing_actor(org(), &room.branch);

    let c_granted = unique("c_cmd_granted");
    let c_withdrawn = unique("c_cmd_withdrawn");

    insert_customer(&room.pool, &room.branch, &c_granted, "Granted User", "G123").await;
    insert_customer(
        &room.pool,
        &room.branch,
        &c_withdrawn,
        "Withdrawn User",
        "W123",
    )
    .await;

    let now = Utc::now();
    insert_tx(
        &room.pool,
        &room.branch,
        &c_granted,
        "Granted User",
        "5000.00",
        now - chrono::Duration::days(5),
    )
    .await;
    insert_tx(
        &room.pool,
        &room.branch,
        &c_withdrawn,
        "Withdrawn User",
        "5000.00",
        now - chrono::Duration::days(5),
    )
    .await;

    // c_granted has active MARKETING consent
    insert_consent(
        &room.pool,
        &c_granted,
        Some("SMS"),
        true,
        now - chrono::Duration::days(9),
    )
    .await;
    // c_withdrawn withdrew consent
    insert_consent(
        &room.pool,
        &c_withdrawn,
        Some("SMS"),
        false,
        now - chrono::Duration::days(8),
    )
    .await;

    // Seed a segment definition
    let def_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO segment_definitions (id, organization_id, name, version, rule, created_by, created_at) \
         VALUES ($1, $2::uuid, 'ConsentGateCmdTest', 1, $3, $4, now())",
    )
    .bind(def_id)
    .bind(ORG)
    .bind(
        serde_json::to_value(SegmentRule {
            tier: None,
            recency_days: None,
            frequency: Some(1),
            value_thb: None,
            home_branch: None,
            lookback_months: Some(12),
        })
        .unwrap(),
    )
    .bind(room.user)
    .execute(&room.pool)
    .await
    .expect("seed segment definition");

    let snap_cmd = SegmentSnapshotCommand;
    let snap_input = SegmentSnapshotInput {
        id: def_id.to_string(),
    };

    let mut tx = room.pool.begin().await.expect("begin tx");
    let out = snap_cmd
        .run(
            &mut tx,
            &room.branch,
            &m_actor,
            snap_input,
            IdempotencyKey::new(Uuid::new_v4()),
        )
        .await
        .expect("segmentSnapshot command succeeds for Marketing actor");
    tx.commit().await.expect("commit tx");

    assert_eq!(
        out.member_count, 1,
        "only the granted customer is snapshotted; withdrawn customer is excluded by D-13 consent gate"
    );

    let granted_in_snap: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(out.id)
    .bind(&c_granted)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(
        granted_in_snap, 1,
        "granted customer is in snapshot members"
    );

    let withdrawn_in_snap: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM segment_snapshot_members WHERE snapshot_id = $1 AND customer_id = $2",
    )
    .bind(out.id)
    .bind(&c_withdrawn)
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);
    assert_eq!(
        withdrawn_in_snap, 0,
        "withdrawn customer is NOT in snapshot members"
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// §8 M3 Schema: 3 tables, UNIQUE (org, name, version), and FK campaign_versions.segment_snapshot_id (Q3)
// -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn m3_schema_creates_tables_unique_version_and_campaign_fk() {
    let room = Room::open().await;

    // 1. Three M3 tables exist
    let table_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema = current_schema() \
         AND table_name IN ('segment_definitions', 'segment_snapshots', 'segment_snapshot_members')",
    )
    .fetch_one(&room.pool)
    .await
    .expect("count M3 tables");

    assert_eq!(
        table_count, 3,
        "MK-6 M3 migration creates its three tables (design §8)"
    );

    // 2. UNIQUE constraint on (organization_id, name, version) exists on segment_definitions
    let unique_cols: Vec<String> = sqlx::query_scalar(
        "SELECT string_agg(att.attname, ',' ORDER BY array_position(con.conkey, att.attnum)) \
         FROM pg_constraint con \
         JOIN pg_class rel ON rel.oid = con.conrelid \
         JOIN pg_namespace nsp ON nsp.oid = rel.relnamespace \
         JOIN pg_attribute att ON att.attrelid = rel.oid AND att.attnum = ANY(con.conkey) \
         WHERE nsp.nspname = current_schema() \
           AND rel.relname = 'segment_definitions' \
           AND con.contype = 'u' \
         GROUP BY con.conname",
    )
    .fetch_all(&room.pool)
    .await
    .unwrap_or_default();

    assert!(
        unique_cols
            .iter()
            .any(|c| c == "organization_id,name,version"),
        "UNIQUE constraint on (organization_id, name, version) exists on segment_definitions: found {unique_cols:?}"
    );

    // 3. FK campaign_versions.segment_snapshot_id -> segment_snapshots(id) (ruling Q3)
    let fk_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.table_constraints tc \
         JOIN information_schema.constraint_column_usage ccu ON ccu.constraint_name = tc.constraint_name \
         WHERE tc.table_schema = current_schema() \
           AND ccu.table_schema = current_schema() \
           AND tc.table_name = 'campaign_versions' \
           AND tc.constraint_type = 'FOREIGN KEY' \
           AND ccu.table_name = 'segment_snapshots'",
    )
    .fetch_one(&room.pool)
    .await
    .unwrap_or(0);

    assert_eq!(
        fk_count, 1,
        "M3 migration adds FK campaign_versions.segment_snapshot_id -> segment_snapshots(id) (ruling Q3)"
    );

    room.finish().await;
}

// -----------------------------------------------------------------------------
// Ruling Q1: Policy rows in areas/segments.rs pin MarketingMembers on EntryPath::Rpc
// -----------------------------------------------------------------------------

#[test]
fn policy_areas_segments_pins_marketing_members_for_all_three_routes() {
    assert_eq!(
        SEGMENT_POLICY_ROWS.len(),
        3,
        "areas/segments.rs must contain exactly 3 policy rows"
    );

    let define_row = SEGMENT_POLICY_ROWS
        .iter()
        .find(|r| r.action == "segmentDefine")
        .expect("segmentDefine row exists");
    assert_eq!(define_row.path, EntryPath::Rpc);
    assert_eq!(
        define_row.need,
        Need::Permission(Permission::MarketingMembers)
    );

    let snap_row = SEGMENT_POLICY_ROWS
        .iter()
        .find(|r| r.action == "segmentSnapshot")
        .expect("segmentSnapshot row exists");
    assert_eq!(snap_row.path, EntryPath::Rpc);
    assert_eq!(
        snap_row.need,
        Need::Permission(Permission::MarketingMembers)
    );

    let preview_row = SEGMENT_POLICY_ROWS
        .iter()
        .find(|r| r.action == "segment_preview")
        .expect("segment_preview row exists");
    assert_eq!(preview_row.path, EntryPath::Rpc);
    assert_eq!(
        preview_row.need,
        Need::Permission(Permission::MarketingMembers)
    );
}
