//! Ticket #1390 (MK-1, design `docs/design/blueprint/marketing-campaigns.md` §9.1 MK-1 row): the campaign version
//! machine, the status derived from the clock, the repository and migration M1.
//!
//! Behaviours locked (ticket §4.1, §9):
//! - a DRAFT edits in place; SIMULATED → edit → DRAFT (D-3);
//! - every status after SIMULATED refuses an edit with `staleVersion` and changes nothing (D-3, design §7);
//! - SCHEDULED / ACTIVE / EXPIRED are derived from `starts_at` / `ends_at` for an APPROVED version only, exactly at
//!   the boundaries too, and are never stored (D-2, §4.2);
//! - approving n+1 supersedes n (APPROVED or PAUSED) and moves `current_version_id`, in the caller's one transaction;
//! - one DRAFT/SIMULATED per campaign (`one_draft_per_campaign`); `ends_at > starts_at`; the `version_hash` CHECK;
//! - M1 creates its six tables (§8) and none of M2's / M3's, and leaves `quotes.benefit_snapshot` as it was.
//!
//! Per the lane controller's ruling on #1390 (claude/ad05bf8d, 2026-10-12) the `ActiveRules` impl is MK-2's and is
//! not tested here.
//!
//! The crate is new, so the scaffold commit before this one declares the items called here with `unimplemented!()`
//! bodies; these tests are red on it by result (a panic or a missing table), not by a compile error.
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each DB test works in a schema of its own.

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_marketing::domain::campaign::campaign::Campaign;
use xc_marketing::domain::campaign::error::CampaignError;
use xc_marketing::domain::campaign::repository::{CampaignRepository, CampaignRepositoryError};
use xc_marketing::domain::campaign::version::{
    CampaignVersion, DerivedStatus, StoredStatus, VersionTerms,
};
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;

const MUST_FINISH: Duration = Duration::from_secs(30);
/// The organisation the migrations seed.
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// A sha256 hex, as submit would write it.
const HASH_A: &str = "a3f1c2d4e5b6978877665544332211ffeeddccbbaa99887766554433221100aa";
const HASH_B: &str = "b3f1c2d4e5b6978877665544332211ffeeddccbbaa99887766554433221100bb";

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

fn terms(budget: &str) -> VersionTerms {
    VersionTerms {
        starts_at: at(2026, 11, 1, 0),
        ends_at: at(2026, 12, 1, 0),
        budget_thb: Decimal::from_str(budget).expect("a decimal"),
        max_uses: Some(100),
        stack_group: "RATE".to_owned(),
        priority: 10,
        segment_snapshot_id: None,
    }
}

fn other_terms() -> VersionTerms {
    VersionTerms {
        starts_at: at(2026, 11, 15, 0),
        ends_at: at(2027, 1, 15, 0),
        budget_thb: Decimal::from_str("2500.50").unwrap(),
        max_uses: None,
        stack_group: "FEE".to_owned(),
        priority: 20,
        segment_snapshot_id: None,
    }
}

fn draft(campaign_id: Uuid, version: i32, created_by: Uuid) -> CampaignVersion {
    CampaignVersion::draft(
        Uuid::new_v4(),
        org(),
        campaign_id,
        version,
        terms("1000.00"),
        created_by,
        at(2026, 10, 20, 9),
    )
}

/// A version moved by hand to `status`, with the hash every status after SIMULATED carries.
fn in_status(status: StoredStatus, hash: &str) -> CampaignVersion {
    let mut v = draft(Uuid::new_v4(), 1, Uuid::new_v4());
    v.status = status;
    if !matches!(status, StoredStatus::Draft | StoredStatus::Simulated) {
        v.version_hash = Some(hash.to_owned());
    }
    if status != StoredStatus::Draft {
        v.simulated_at = Some(at(2026, 10, 20, 10));
    }
    v
}

// ---------------------------------------------------------------- domain: the version machine (no database)

#[test]
fn a_draft_edits_in_place() {
    let mut v = draft(Uuid::new_v4(), 1, Uuid::new_v4());
    let before = v.clone();
    let now = at(2026, 10, 21, 8);

    v.edit(other_terms(), now)
        .expect("a DRAFT accepts an edit (D-3)");

    assert_eq!(v.id, before.id, "the same version row, not a new one");
    assert_eq!(
        v.version, before.version,
        "the version number does not move"
    );
    assert_eq!(v.campaign_id, before.campaign_id);
    assert_eq!(v.status, StoredStatus::Draft, "a DRAFT stays DRAFT");
    assert_eq!(v.terms, other_terms(), "the edit is applied");
    assert_eq!(
        v.edited_at, now,
        "edited_at is the edit's time (D-4 compares it)"
    );
}

#[test]
fn a_simulated_version_returns_to_draft_on_edit() {
    let mut v = in_status(StoredStatus::Simulated, HASH_A);
    let before = v.clone();

    v.edit(other_terms(), at(2026, 10, 21, 8))
        .expect("a SIMULATED version accepts an edit (§4.2 SIMULATED --edit--> DRAFT)");

    assert_eq!(v.status, StoredStatus::Draft, "SIMULATED → edit → DRAFT");
    assert_eq!(v.id, before.id, "edited in place");
    assert_eq!(v.version, before.version);
    assert_eq!(v.terms, other_terms());
}

#[test]
fn every_status_after_simulated_refuses_an_edit_with_stale_version() {
    for status in [
        StoredStatus::PendingApproval,
        StoredStatus::Approved,
        StoredStatus::Rejected,
        StoredStatus::Paused,
        StoredStatus::Superseded,
    ] {
        let mut v = in_status(status, HASH_A);
        let before = v.clone();
        let got = v.edit(other_terms(), at(2026, 10, 21, 8));
        assert_eq!(
            got,
            Err(CampaignError::StaleVersion),
            "{status:?} is frozen: an edit is `staleVersion` (D-3, design §7)"
        );
        assert_eq!(v, before, "a refused edit of {status:?} changes nothing");
    }
}

#[test]
fn derived_status_follows_the_clock_for_an_approved_version() {
    let v = in_status(StoredStatus::Approved, HASH_A);
    let starts = v.terms.starts_at;
    let ends = v.terms.ends_at;
    let micro = chrono::Duration::microseconds(1);

    let cases = [
        (starts - chrono::Duration::days(3), DerivedStatus::Scheduled),
        (starts - micro, DerivedStatus::Scheduled),
        (starts, DerivedStatus::Active),
        (starts + chrono::Duration::days(10), DerivedStatus::Active),
        (ends - micro, DerivedStatus::Active),
        (ends, DerivedStatus::Expired),
        (ends + chrono::Duration::days(40), DerivedStatus::Expired),
    ];
    for (now, want) in cases {
        assert_eq!(
            v.derived_status(now),
            Some(want),
            "APPROVED at {now} (starts {starts}, ends {ends}): SCHEDULED if now < starts_at, ACTIVE if \
             starts_at ≤ now < ends_at, EXPIRED if now ≥ ends_at (D-2)"
        );
    }
}

#[test]
fn only_an_approved_version_has_a_derived_status() {
    for status in [
        StoredStatus::Draft,
        StoredStatus::Simulated,
        StoredStatus::PendingApproval,
        StoredStatus::Rejected,
        StoredStatus::Paused,
        StoredStatus::Superseded,
    ] {
        let v = in_status(status, HASH_A);
        let inside = v.terms.starts_at + chrono::Duration::days(1);
        assert_eq!(
            v.derived_status(inside),
            None,
            "{status:?} has no derived status, even inside its dates (D-2: derived for APPROVED only)"
        );
    }
}

// ---------------------------------------------------------------- database harness

struct Room {
    pool: PgPool,
    schema: String,
    user: Uuid,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("mk1_campaigns");
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
            .max_connections(5)
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
        let user: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, 'marketing', 'ACTIVE', 'mk1-test') RETURNING id::text",
        )
        .bind(ORG)
        .fetch_one(&pool)
        .await
        .expect("a MARKETING user");
        Room {
            pool,
            schema,
            user: Uuid::parse_str(&user).unwrap(),
        }
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    async fn tx(&self) -> Transaction<'static, Postgres> {
        self.pool.begin().await.expect("begin")
    }

    fn campaign(&self) -> Campaign {
        Campaign {
            id: Uuid::new_v4(),
            organization_id: org(),
            name: unique("campaign"),
            current_version_id: None,
            created_by: self.user,
            created_at: at(2026, 10, 20, 9),
        }
    }

    /// A campaign and its version 1 DRAFT, committed through the repository.
    async fn campaign_with_draft(
        &self,
        repo: &PgCampaignRepository,
    ) -> (Campaign, CampaignVersion) {
        let campaign = self.campaign();
        let v1 = draft(campaign.id, 1, self.user);
        let mut tx = self.tx().await;
        repo.insert_campaign(&mut tx, &campaign)
            .await
            .expect("insert the campaign");
        repo.insert_version(&mut tx, &v1)
            .await
            .expect("insert version 1 DRAFT");
        tx.commit().await.expect("commit");
        (campaign, v1)
    }

    /// Inserts version `version` of `campaign` already PENDING_APPROVAL (as MK-4's submit leaves it).
    async fn pending(
        &self,
        repo: &PgCampaignRepository,
        campaign: &Campaign,
        version: i32,
        hash: &str,
    ) -> CampaignVersion {
        let mut v = draft(campaign.id, version, self.user);
        v.status = StoredStatus::PendingApproval;
        v.version_hash = Some(hash.to_owned());
        v.simulated_at = Some(at(2026, 10, 20, 10));
        let mut tx = self.tx().await;
        repo.insert_version(&mut tx, &v)
            .await
            .expect("insert a PENDING_APPROVAL version");
        tx.commit().await.expect("commit");
        v
    }

    async fn approve(&self, repo: &PgCampaignRepository, version_id: Uuid) {
        let mut tx = self.tx().await;
        repo.approve(&mut tx, org(), version_id)
            .await
            .expect("approve");
        tx.commit().await.expect("commit the approval");
    }

    async fn status_of(&self, version_id: Uuid) -> String {
        sqlx::query_scalar("SELECT status FROM campaign_versions WHERE id = $1::uuid")
            .bind(version_id.to_string())
            .fetch_one(&self.pool)
            .await
            .expect("the version row")
    }

    async fn current_of(&self, campaign_id: Uuid) -> Option<Uuid> {
        let current: Option<String> = sqlx::query_scalar(
            "SELECT current_version_id::text FROM campaigns WHERE id = $1::uuid",
        )
        .bind(campaign_id.to_string())
        .fetch_one(&self.pool)
        .await
        .expect("the campaign row");
        current.map(|s| Uuid::parse_str(&s).unwrap())
    }

    /// A bare campaigns row for the schema tests (design §4.1 columns).
    async fn bare_campaign(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
             VALUES ($1::uuid, $2::uuid, $3, $4::uuid, now())",
        )
        .bind(id.to_string())
        .bind(ORG)
        .bind(unique("bare"))
        .bind(self.user.to_string())
        .execute(&self.pool)
        .await
        .expect("a campaigns row");
        id
    }

    /// Inserts a campaign_versions row by SQL (design §4.1 columns) and returns the database's answer.
    async fn raw_version(
        &self,
        campaign_id: Uuid,
        version: i32,
        status: &str,
        hash: Option<&str>,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
             VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, $7, 1000.00, 'RATE', 10, $8, now(), $9::uuid, now())",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(ORG)
        .bind(campaign_id.to_string())
        .bind(version)
        .bind(status)
        .bind(starts_at)
        .bind(ends_at)
        .bind(hash)
        .bind(self.user.to_string())
        .execute(&self.pool)
        .await
        .map(|_| ())
    }
}

fn sql_state(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
}

fn constraint_of(err: &CampaignRepositoryError) -> Option<String> {
    let CampaignRepositoryError::Database(e) = err;
    e.as_database_error()
        .and_then(|d| d.constraint())
        .map(str::to_owned)
}

// ---------------------------------------------------------------- migration M1 (schema)

#[tokio::test]
async fn m1_creates_its_six_tables_and_none_of_m2_or_m3() {
    let room = Room::start().await;
    let present = |table: &'static str| {
        let pool = room.pool.clone();
        let schema = room.schema.clone();
        async move {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM information_schema.tables WHERE table_schema = $1 AND table_name = $2",
            )
            .bind(schema)
            .bind(table)
            .fetch_one(&pool)
            .await
            .expect("information_schema");
            n == 1
        }
    };
    for table in [
        "campaigns",
        "campaign_versions",
        "campaign_branches",
        "benefit_rules",
        "campaign_simulations",
        "margin_guards",
    ] {
        assert!(present(table).await, "M1 creates `{table}` (design §8)");
    }
    for table in [
        "coupons",
        "campaign_budgets",
        "benefit_reservations",
        "benefit_redemptions",
        "segment_definitions",
        "segment_snapshots",
        "segment_snapshot_members",
    ] {
        assert!(
            !present(table).await,
            "`{table}` belongs to M2 (MK-3) or M3 (MK-6), not M1 (design §8)"
        );
    }

    let money: Vec<(String, String, i32, i32)> = sqlx::query_as(
        "SELECT table_name::text, column_name::text, numeric_precision, numeric_scale \
         FROM information_schema.columns WHERE table_schema = $1 \
           AND (table_name, column_name) IN (('campaign_versions','budget_thb'), ('margin_guards','max_spread_share')) \
         ORDER BY table_name",
    )
    .bind(&room.schema)
    .fetch_all(&room.pool)
    .await
    .expect("information_schema columns");
    assert_eq!(
        money,
        vec![
            (
                "campaign_versions".to_owned(),
                "budget_thb".to_owned(),
                20,
                2
            ),
            (
                "margin_guards".to_owned(),
                "max_spread_share".to_owned(),
                5,
                4
            ),
        ],
        "money is numeric(20,2), never a float; the margin share numeric(5,4) (§4.1)"
    );

    let snapshot: (String, Option<String>) = sqlx::query_as(
        "SELECT is_nullable::text, column_default::text FROM information_schema.columns \
         WHERE table_schema = $1 AND table_name = 'quotes' AND column_name = 'benefit_snapshot'",
    )
    .bind(&room.schema)
    .fetch_one(&room.pool)
    .await
    .expect("quotes.benefit_snapshot");
    assert_eq!(
        snapshot.0, "NO",
        "quotes.benefit_snapshot stays NOT NULL (§8 item 1)"
    );
    assert!(
        snapshot.1.as_deref().unwrap_or("").starts_with("'{}'"),
        "quotes.benefit_snapshot keeps its '{{}}' default (§8 item 1); got {:?}",
        snapshot.1
    );
    room.finish().await;
}

#[tokio::test]
async fn a_stored_status_is_one_of_the_seven_and_never_a_derived_one() {
    let room = Room::start().await;
    let (s, e) = (at(2026, 11, 1, 0), at(2026, 12, 1, 0));
    for (status, hash) in [
        ("DRAFT", None),
        ("SIMULATED", None),
        ("PENDING_APPROVAL", Some(HASH_A)),
        ("APPROVED", Some(HASH_A)),
        ("REJECTED", Some(HASH_A)),
        ("PAUSED", Some(HASH_A)),
        ("SUPERSEDED", Some(HASH_A)),
    ] {
        let campaign = room.bare_campaign().await;
        room.raw_version(campaign, 1, status, hash, s, e)
            .await
            .unwrap_or_else(|err| panic!("{status} is a stored status (D-2): {err}"));
    }
    for status in ["SCHEDULED", "ACTIVE", "EXPIRED"] {
        let campaign = room.bare_campaign().await;
        let err = room
            .raw_version(campaign, 1, status, Some(HASH_A), s, e)
            .await
            .expect_err("a derived status is never stored (D-2)");
        assert_eq!(
            sql_state(&err).as_deref(),
            Some("23514"),
            "{status} is refused by the status CHECK: {err}"
        );
    }
    room.finish().await;
}

#[tokio::test]
async fn ends_at_not_after_starts_at_is_refused() {
    let room = Room::start().await;
    let s = at(2026, 11, 1, 0);
    for (label, ends) in [("equal", s), ("before", s - chrono::Duration::hours(1))] {
        let campaign = room.bare_campaign().await;
        let err = room
            .raw_version(campaign, 1, "DRAFT", None, s, ends)
            .await
            .expect_err("ends_at ≤ starts_at is refused");
        assert_eq!(
            sql_state(&err).as_deref(),
            Some("23514"),
            "ends_at {label} starts_at: the CHECK (ends_at > starts_at) refuses it: {err}"
        );
    }
    room.finish().await;
}

#[tokio::test]
async fn version_hash_is_present_exactly_from_pending_approval_on() {
    let room = Room::start().await;
    let (s, e) = (at(2026, 11, 1, 0), at(2026, 12, 1, 0));
    for (status, hash, why) in [
        ("DRAFT", Some(HASH_A), "a DRAFT has no hash yet"),
        (
            "SIMULATED",
            Some(HASH_A),
            "a SIMULATED version has no hash yet",
        ),
        ("PENDING_APPROVAL", None, "submit writes the hash"),
        ("APPROVED", None, "approval binds the hash"),
        ("PAUSED", None, "a PAUSED version keeps its hash"),
    ] {
        let campaign = room.bare_campaign().await;
        let err = room
            .raw_version(campaign, 1, status, hash, s, e)
            .await
            .expect_err(why);
        assert_eq!(
            sql_state(&err).as_deref(),
            Some("23514"),
            "{status} with hash {hash:?}: {why} (§4.1 CHECK): {err}"
        );
    }
    room.finish().await;
}

// ---------------------------------------------------------------- repository

#[tokio::test]
async fn one_draft_per_campaign() {
    let room = Room::start().await;
    let repo = PgCampaignRepository::new();
    let (campaign, mut v1) = room.campaign_with_draft(&repo).await;

    for status in [StoredStatus::Draft, StoredStatus::Simulated] {
        let mut v2 = draft(campaign.id, 2, room.user);
        v2.status = status;
        let mut tx = room.tx().await;
        let err = repo
            .insert_version(&mut tx, &v2)
            .await
            .expect_err("a second DRAFT/SIMULATED for one campaign is refused");
        assert_eq!(
            constraint_of(&err).as_deref(),
            Some("one_draft_per_campaign"),
            "a second {status:?} beside version 1 DRAFT: the partial unique index refuses it: {err}"
        );
        tx.rollback().await.expect("rollback");
    }

    // another campaign's draft is its own
    room.campaign_with_draft(&repo).await;

    // once version 1 is submitted, a new DRAFT is allowed
    v1.status = StoredStatus::PendingApproval;
    v1.version_hash = Some(HASH_A.to_owned());
    v1.simulated_at = Some(at(2026, 10, 20, 10));
    let mut tx = room.tx().await;
    repo.save_version(&mut tx, &v1)
        .await
        .expect("version 1 → PENDING_APPROVAL");
    let v2 = draft(campaign.id, 2, room.user);
    repo.insert_version(&mut tx, &v2)
        .await
        .expect("with no DRAFT/SIMULATED left, version 2 DRAFT is accepted");
    tx.commit().await.expect("commit");
    room.finish().await;
}

#[tokio::test]
async fn a_saved_draft_edit_writes_the_same_row() {
    let room = Room::start().await;
    let repo = PgCampaignRepository::new();
    let (campaign, mut v1) = room.campaign_with_draft(&repo).await;

    v1.edit(other_terms(), at(2026, 10, 21, 8))
        .expect("a DRAFT edits");
    let mut tx = room.tx().await;
    repo.save_version(&mut tx, &v1)
        .await
        .expect("save the edited draft");
    tx.commit().await.expect("commit");

    let rows: Vec<(String, i32, String, String, String)> = sqlx::query_as(
        "SELECT id::text, version, status, budget_thb::text, stack_group \
         FROM campaign_versions WHERE campaign_id = $1::uuid",
    )
    .bind(campaign.id.to_string())
    .fetch_all(&room.pool)
    .await
    .expect("the campaign's versions");
    assert_eq!(
        rows,
        vec![(
            v1.id.to_string(),
            1,
            "DRAFT".to_owned(),
            "2500.50".to_owned(),
            "FEE".to_owned()
        )],
        "a DRAFT edit is written in place: one row, same id and number, the new terms (D-3)"
    );

    let mut tx = room.tx().await;
    let read = repo
        .find_version(&mut tx, org(), v1.id)
        .await
        .expect("find")
        .expect("the version exists");
    tx.rollback().await.expect("rollback");
    assert_eq!(read.status, StoredStatus::Draft);
    assert_eq!(
        read.terms,
        other_terms(),
        "the repository reads back what it wrote"
    );
    room.finish().await;
}

#[tokio::test]
async fn a_saved_edit_of_a_simulated_version_stores_draft() {
    let room = Room::start().await;
    let repo = PgCampaignRepository::new();
    let (_campaign, mut v1) = room.campaign_with_draft(&repo).await;

    v1.status = StoredStatus::Simulated;
    v1.simulated_at = Some(at(2026, 10, 20, 10));
    let mut tx = room.tx().await;
    repo.save_version(&mut tx, &v1)
        .await
        .expect("version 1 → SIMULATED");
    tx.commit().await.expect("commit");
    assert_eq!(room.status_of(v1.id).await, "SIMULATED");

    let mut tx = room.tx().await;
    let mut read = repo
        .find_version(&mut tx, org(), v1.id)
        .await
        .expect("find")
        .expect("the version exists");
    assert_eq!(read.status, StoredStatus::Simulated, "SIMULATED reads back");
    read.edit(other_terms(), at(2026, 10, 21, 8))
        .expect("a SIMULATED version edits");
    repo.save_version(&mut tx, &read)
        .await
        .expect("save the edit");
    tx.commit().await.expect("commit");

    assert_eq!(
        room.status_of(v1.id).await,
        "DRAFT",
        "SIMULATED → edit → DRAFT is stored (§4.2)"
    );
    room.finish().await;
}

#[tokio::test]
async fn approving_n_plus_1_supersedes_n_and_moves_the_current_version() {
    let room = Room::start().await;
    let repo = PgCampaignRepository::new();
    let campaign = room.campaign();
    let mut tx = room.tx().await;
    repo.insert_campaign(&mut tx, &campaign)
        .await
        .expect("insert the campaign");
    tx.commit().await.expect("commit");

    let v1 = room.pending(&repo, &campaign, 1, HASH_A).await;
    room.approve(&repo, v1.id).await;
    assert_eq!(room.status_of(v1.id).await, "APPROVED");
    assert_eq!(
        room.current_of(campaign.id).await,
        Some(v1.id),
        "the first approval sets current_version_id"
    );

    // a second campaign whose APPROVED version must not move
    let bystander = room.campaign();
    let mut tx = room.tx().await;
    repo.insert_campaign(&mut tx, &bystander)
        .await
        .expect("insert the bystander");
    tx.commit().await.expect("commit");
    let b1 = room.pending(&repo, &bystander, 1, HASH_B).await;
    room.approve(&repo, b1.id).await;

    let v2 = room.pending(&repo, &campaign, 2, HASH_B).await;
    room.approve(&repo, v2.id).await;

    assert_eq!(room.status_of(v2.id).await, "APPROVED", "n+1 is APPROVED");
    assert_eq!(
        room.status_of(v1.id).await,
        "SUPERSEDED",
        "approving n+1 supersedes n (D-3)"
    );
    assert_eq!(
        room.current_of(campaign.id).await,
        Some(v2.id),
        "current_version_id moves to n+1"
    );
    assert_eq!(
        room.status_of(b1.id).await,
        "APPROVED",
        "another campaign's APPROVED version is untouched"
    );
    assert_eq!(room.current_of(bystander.id).await, Some(b1.id));

    let mut tx = room.tx().await;
    let read = repo
        .find_campaign(&mut tx, org(), campaign.id)
        .await
        .expect("find")
        .expect("the campaign exists");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        read.current_version_id,
        Some(v2.id),
        "the repository reads the move"
    );
    room.finish().await;
}

#[tokio::test]
async fn approving_n_plus_1_while_n_is_paused_supersedes_n() {
    let room = Room::start().await;
    let repo = PgCampaignRepository::new();
    let campaign = room.campaign();
    let mut tx = room.tx().await;
    repo.insert_campaign(&mut tx, &campaign)
        .await
        .expect("insert the campaign");
    tx.commit().await.expect("commit");

    let mut v1 = room.pending(&repo, &campaign, 1, HASH_A).await;
    room.approve(&repo, v1.id).await;
    v1.status = StoredStatus::Paused;
    let mut tx = room.tx().await;
    repo.save_version(&mut tx, &v1)
        .await
        .expect("version 1 → PAUSED");
    tx.commit().await.expect("commit");
    assert_eq!(room.status_of(v1.id).await, "PAUSED");

    let v2 = room.pending(&repo, &campaign, 2, HASH_B).await;
    room.approve(&repo, v2.id).await;

    assert_eq!(
        room.status_of(v1.id).await,
        "SUPERSEDED",
        "a PAUSED n is superseded by approving n+1 (§4.2, ticket §9)"
    );
    assert_eq!(room.status_of(v2.id).await, "APPROVED");
    assert_eq!(room.current_of(campaign.id).await, Some(v2.id));
    room.finish().await;
}

#[tokio::test]
async fn the_approval_is_one_transaction() {
    let room = Room::start().await;
    let repo = PgCampaignRepository::new();
    let campaign = room.campaign();
    let mut tx = room.tx().await;
    repo.insert_campaign(&mut tx, &campaign)
        .await
        .expect("insert the campaign");
    tx.commit().await.expect("commit");
    let v1 = room.pending(&repo, &campaign, 1, HASH_A).await;
    room.approve(&repo, v1.id).await;
    let v2 = room.pending(&repo, &campaign, 2, HASH_B).await;

    let unchanged = |why: &'static str| {
        let room = &room;
        async move {
            assert_eq!(
                room.status_of(v1.id).await,
                "APPROVED",
                "{why}: n is still APPROVED"
            );
            assert_eq!(
                room.status_of(v2.id).await,
                "PENDING_APPROVAL",
                "{why}: n+1 is still PENDING_APPROVAL"
            );
            assert_eq!(
                room.current_of(campaign.id).await,
                Some(v1.id),
                "{why}: current_version_id has not moved"
            );
        }
    };

    // every write goes through the caller's transaction: nothing shows before commit, nothing stays after rollback
    let mut tx = room.tx().await;
    repo.approve(&mut tx, org(), v2.id)
        .await
        .expect("approve n+1");
    unchanged("before the caller commits").await;
    tx.rollback().await.expect("rollback");
    unchanged("after the caller rolls back").await;

    // a failing write inside the approval is an error, never a silent partial approval
    for (table, trigger) in [
        ("campaigns", "mk1_fail_campaigns"),
        ("campaign_versions", "mk1_fail_versions"),
    ] {
        sqlx::query(
            "CREATE OR REPLACE FUNCTION mk1_fail() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'mk1 test: refuse this write'; END $$",
        )
        .execute(&room.pool)
        .await
        .expect("the failing trigger function");
        sqlx::query(AssertSqlSafe(format!(
            "CREATE TRIGGER {trigger} BEFORE UPDATE ON {table} FOR EACH ROW EXECUTE FUNCTION mk1_fail()"
        )))
        .execute(&room.pool)
        .await
        .expect("the failing trigger");
        let mut tx = room.tx().await;
        let got = repo.approve(&mut tx, org(), v2.id).await;
        assert!(
            matches!(got, Err(CampaignRepositoryError::Database(_))),
            "an UPDATE of {table} fails inside approve: approve must answer the database error, got {got:?}"
        );
        tx.rollback().await.expect("rollback");
        sqlx::query(AssertSqlSafe(format!("DROP TRIGGER {trigger} ON {table}")))
            .execute(&room.pool)
            .await
            .expect("drop the failing trigger");
        unchanged("after a failed approval").await;
    }

    room.approve(&repo, v2.id).await;
    assert_eq!(room.status_of(v1.id).await, "SUPERSEDED");
    assert_eq!(room.status_of(v2.id).await, "APPROVED");
    assert_eq!(room.current_of(campaign.id).await, Some(v2.id));
    room.finish().await;
}
