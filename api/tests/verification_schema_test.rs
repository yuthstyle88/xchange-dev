//! VF-1 (#1386): migration M1 `api/migrations/<version>_verification.sql` (design
//! `docs/design/blueprint/verification.md` §4.1, §8 M1, §9.1 row "VF-1").
//!
//! Written before the implementation by the ticket author (claude/29df2ec5). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! The lane controller's rulings on #1386 (claude/ad05bf8d, 2026-10-12): REJECTED, NEEDS_UPDATE and REVOKED each
//! need a `decision_reason_code` (design §4.1), and since S-5 was decided (PR 1438) the code is one of six values,
//! enforced by a `CHECK`; no free text is stored.
//!
//! M1 is found by its file name: the one migration whose description is exactly `verification` (§5 File scope,
//! `VERSION_verification.sql`), whatever version the lane controller allocates. On `main` there is none, so every
//! test here is red by result (M1 missing, or its tables absent), not by a compile error.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.
//! Each test works in a schema of its own.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

const MUST_FINISH: Duration = Duration::from_secs(30);
/// The organisation the migrations seed.
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// §8 M1 item 3: M1 comes after these two (both on `main`).
const ORGANIZATIONS_MIGRATION: i64 = 20261011100000;
const CUSTOMERS_ORG_MIGRATION: i64 = 20261019100000;
/// `MAX_UPLOAD_BYTES`, `api/src/infrastructure/files/file_service.rs:12`.
const MAX_UPLOAD_BYTES: i64 = 15_728_640;
const THE_THREE: [&str; 3] = [
    "verification_policies",
    "verification_cases",
    "verification_documents",
];
const OPEN: [&str; 4] = ["DRAFT", "SUBMITTED", "UNDER_REVIEW", "NEEDS_UPDATE"];
const CLOSED: [&str; 4] = ["VERIFIED", "REJECTED", "EXPIRED", "REVOKED"];
/// The statuses that need a reason code (design §4.1).
const REASONED: [&str; 3] = ["REJECTED", "NEEDS_UPDATE", "REVOKED"];
/// S-5 (owner 2026-10-12, PR 1438): the only values `decision_reason_code` may hold.
const REASON_CODES: [&str; 6] = [
    "DOCUMENT_UNREADABLE",
    "DOCUMENT_EXPIRED",
    "DETAILS_MISMATCH",
    "DOCUMENT_NOT_ACCEPTED",
    "SUSPECTED_FRAUD",
    "OTHER",
];

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// The SQLSTATE and constraint name of a refused statement.
struct Refusal {
    code: String,
    constraint: Option<String>,
    message: String,
}

fn refusal(result: Result<(), sqlx::Error>, what: &str) -> Refusal {
    match result {
        Ok(()) => panic!("{what}: was accepted, the schema must refuse it"),
        Err(sqlx::Error::Database(e)) => Refusal {
            code: e.code().map(|c| c.into_owned()).unwrap_or_default(),
            constraint: e.constraint().map(str::to_owned),
            message: e.message().to_owned(),
        },
        Err(other) => panic!("{what}: failed without a database refusal: {other}"),
    }
}

fn assert_check_violation(result: Result<(), sqlx::Error>, what: &str) {
    let r = refusal(result, what);
    assert_eq!(
        r.code, "23514",
        "{what}: a CHECK violation (23514), got {} {:?}: {}",
        r.code, r.constraint, r.message
    );
}

/// One `verification_documents` row to insert.
struct Doc<'a> {
    mime: &'a str,
    size: i64,
    object_ref: Option<String>,
    removed: bool,
    purged: bool,
}

impl<'a> Doc<'a> {
    /// A stored document: a fresh object ref, neither removed nor purged.
    fn stored(mime: &'a str, size: i64) -> Self {
        Doc {
            mime,
            size,
            object_ref: Some(unique("obj")),
            removed: false,
            purged: false,
        }
    }
}

struct Db {
    pool: PgPool,
    schema: String,
    migrator: Migrator,
}

impl Db {
    /// A schema with no migration applied yet.
    async fn empty() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("vf1");
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
            .max_connections(3)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrator = Migrator::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))
            .await
            .expect("read api/migrations");
        Db {
            pool,
            schema,
            migrator,
        }
    }

    /// Every migration applied, in version order, on a fresh schema.
    async fn fresh() -> Db {
        let db = Db::empty().await;
        db.migrator
            .run(&db.pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        db
    }

    /// The version of M1: the migration named `<version>_verification.sql`.
    fn m1(&self) -> i64 {
        let found: Vec<i64> = self
            .migrator
            .iter()
            .filter(|m| m.description == "verification")
            .map(|m| m.version)
            .collect();
        assert_eq!(
            found.len(),
            1,
            "exactly one migration api/migrations/<version>_verification.sql (VF-1 M1), found {found:?}"
        );
        found[0]
    }

    async fn finish(self) {
        let Db { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    async fn exec(&self, sql: &str) -> Result<(), sqlx::Error> {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    async fn ok(&self, sql: &str) {
        self.exec(sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn text(&self, sql: &str) -> String {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// An organisation of its own (code and timezone are NOT NULL).
    async fn organisation(&self) -> Uuid {
        let id = Uuid::new_v4();
        self.ok(&format!(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ('{id}', '{}', 'Asia/Bangkok', now())",
            unique("org")
        ))
        .await;
        id
    }

    async fn branch(&self, org: &str) -> String {
        let id = unique("B");
        self.ok(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{id}', '{id}', '{org}'::uuid)"
        ))
        .await;
        id
    }

    async fn user(&self, org: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.ok(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{id}', '{org}'::uuid, 'Reviewer', 'ACTIVE', 'test')"
        ))
        .await;
        id
    }

    async fn customer(&self, org: &str) -> String {
        let id = unique("CX");
        self.ok(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('{id}', '{org}'::uuid, '{id}', 'Somchai', 'LIVE', now())"
        ))
        .await;
        id
    }

    /// A case of ORG under policy version 1 (seeded by M1). `reviewed` sets `reviewed_by` and `decided_at`.
    async fn case(
        &self,
        customer: &str,
        status: &str,
        reviewer: Option<Uuid>,
        decided: bool,
        reason: Option<&str>,
    ) -> Result<Uuid, sqlx::Error> {
        let id = Uuid::new_v4();
        let reviewed_by = reviewer.map_or("NULL".to_owned(), |r| format!("'{r}'::uuid"));
        let decided_at = if decided { "now()" } else { "NULL" };
        let reason = reason.map_or("NULL".to_owned(), |r| format!("'{r}'"));
        self.exec(&format!(
            "INSERT INTO verification_cases (id, organization_id, customer_id, status, policy_version, \
             submitted_at, decided_at, reviewed_by, decision_reason_code, created_at) \
             VALUES ('{id}', '{ORG}'::uuid, '{customer}', '{status}', 1, now(), {decided_at}, {reviewed_by}, \
             {reason}, now())"
        ))
        .await
        .map(|()| id)
    }

    /// A case whose columns satisfy every check for its status.
    async fn valid_case(&self, customer: &str, status: &str, reviewer: Uuid) -> Uuid {
        let decided = !matches!(status, "DRAFT" | "SUBMITTED" | "UNDER_REVIEW");
        let reviewed = decided.then_some(reviewer);
        let reason = matches!(status, "REJECTED" | "NEEDS_UPDATE" | "REVOKED").then_some("OTHER");
        self.case(customer, status, reviewed, decided, reason)
            .await
            .unwrap_or_else(|e| panic!("a valid {status} case: {e}"))
    }

    async fn document(&self, case: Uuid, uploader: Uuid, doc: Doc<'_>) -> Result<(), sqlx::Error> {
        let Doc {
            mime,
            size,
            object_ref,
            removed,
            purged,
        } = doc;
        let object_ref = object_ref.map_or("NULL".to_owned(), |r| format!("'{r}'"));
        let removed_at = if removed { "now()" } else { "NULL" };
        let purged_at = if purged { "now()" } else { "NULL" };
        self.exec(&format!(
            "INSERT INTO verification_documents (id, organization_id, case_id, document_type, \
             encrypted_object_ref, checksum, mime, size_bytes, uploaded_at, uploaded_by, removed_at, purged_at, \
             created_at) \
             VALUES ('{}', '{ORG}'::uuid, '{case}', 'PASSPORT', {object_ref}, \
             sha256('x'::bytea), '{mime}', {size}, now(), '{uploader}'::uuid, {removed_at}, {purged_at}, now())",
            Uuid::new_v4()
        ))
        .await
    }

    /// Columns (name, type, nullability, default) and constraints of a table, as one comparable text.
    async fn shape(&self, table: &str) -> String {
        let columns = self
            .text(&format!(
                "SELECT coalesce(string_agg(column_name || ' ' || data_type || ' ' || is_nullable || ' ' \
                 || coalesce(column_default, '-'), E'\\n' ORDER BY column_name), '') \
                 FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = '{table}'"
            ))
            .await;
        let constraints = self
            .text(&format!(
                "SELECT coalesce(string_agg(conname || ' ' || pg_get_constraintdef(oid), E'\\n' \
                 ORDER BY conname), '') FROM pg_constraint \
                 WHERE conrelid = to_regclass('{table}')"
            ))
            .await;
        assert!(!columns.is_empty(), "{table} exists before M1");
        format!("{columns}\n--\n{constraints}")
    }

    /// Row count and an md5 over every row of a table, in key order.
    async fn rows(&self, table: &str) -> String {
        self.text(&format!(
            "SELECT count(*)::text || ' ' || coalesce(md5(string_agg(t::text, '|' ORDER BY t::text)), '-') \
             FROM {table} t"
        ))
        .await
    }
}

#[tokio::test]
async fn a_fresh_database_applies_every_migration_in_version_order_with_m1_after_its_dependencies()
{
    let db = Db::fresh().await;
    let m1 = db.m1();
    assert!(
        m1 > ORGANIZATIONS_MIGRATION.max(CUSTOMERS_ORG_MIGRATION),
        "§8 M1 item 3: M1 ({m1}) comes after 20261011100000_organizations and 20261019100000_customers_org"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM _sqlx_migrations WHERE success")
            .await,
        db.migrator.iter().count() as i64,
        "every migration file is applied and succeeded"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM _sqlx_migrations WHERE version = {m1} AND success"
        ))
        .await,
        1,
        "M1 is applied"
    );
    for table in THE_THREE {
        assert_eq!(
            db.count(&format!(
                "SELECT count(*) FROM information_schema.tables \
                 WHERE table_schema = current_schema() AND table_name = '{table}'"
            ))
            .await,
            1,
            "{table} exists after M1"
        );
    }
    for (index, partial) in [
        (
            "verification_one_open_case",
            "WHERE (status = ANY (ARRAY['DRAFT'::text, 'SUBMITTED'::text, 'UNDER_REVIEW'::text, 'NEEDS_UPDATE'::text]))",
        ),
        (
            "verification_queue",
            "WHERE (status = ANY (ARRAY['SUBMITTED'::text, 'UNDER_REVIEW'::text]))",
        ),
    ] {
        let def = db
            .text(&format!(
                "SELECT coalesce((SELECT indexdef FROM pg_indexes \
                 WHERE schemaname = current_schema() AND tablename = 'verification_cases' \
                 AND indexname = '{index}'), '')"
            ))
            .await;
        assert!(
            def.contains(partial),
            "design §4.1: index {index} on verification_cases with `{partial}`, got `{def}`"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn no_column_of_the_three_tables_is_bytea_except_the_checksum() {
    let db = Db::fresh().await;
    for table in THE_THREE {
        assert!(
            db.count(&format!(
                "SELECT count(*) FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = '{table}'"
            ))
            .await
                > 0,
            "{table} has columns"
        );
    }
    let bytea: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text || '.' || column_name::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = ANY($1) AND data_type = 'bytea' ORDER BY 1",
    )
    .bind(THE_THREE.as_slice())
    .fetch_all(&db.pool)
    .await
    .expect("read the bytea columns");
    assert_eq!(
        bytea,
        vec!["verification_documents.checksum".to_owned()],
        "Blueprint §16 / design §4.1: no document image in the database; the only bytea is the SHA-256 checksum"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_second_open_case_for_one_customer_is_refused_by_verification_one_open_case() {
    let db = Db::fresh().await;
    let reviewer = db.user(ORG).await;
    for first in OPEN {
        for second in OPEN {
            let customer = db.customer(ORG).await;
            db.valid_case(&customer, first, reviewer).await;
            let decided = second == "NEEDS_UPDATE";
            let r = refusal(
                db.case(
                    &customer,
                    second,
                    decided.then_some(reviewer),
                    decided,
                    decided.then_some("OTHER"),
                )
                .await
                .map(|_| ()),
                &format!("a {second} case beside an open {first} case of the same customer"),
            );
            assert_eq!(r.code, "23505", "a unique violation: {}", r.message);
            assert_eq!(
                r.constraint.as_deref(),
                Some("verification_one_open_case"),
                "refused by the partial unique index of design §4.1 (V14)"
            );
        }
    }
    db.finish().await;
}

#[tokio::test]
async fn a_closed_case_does_not_count_against_the_one_open_case_index() {
    let db = Db::fresh().await;
    let reviewer = db.user(ORG).await;
    for closed in CLOSED {
        let customer = db.customer(ORG).await;
        db.valid_case(&customer, closed, reviewer).await;
        db.valid_case(&customer, closed, reviewer).await;
        db.case(&customer, "DRAFT", None, false, None)
            .await
            .unwrap_or_else(|e| panic!("a new DRAFT beside two {closed} cases: {e}"));
    }
    db.finish().await;
}

#[tokio::test]
async fn rejected_needs_update_and_revoked_without_a_reason_are_refused() {
    // Design §4.1: a decided case that ends badly or asks for an update carries a reason code.
    let db = Db::fresh().await;
    let reviewer = db.user(ORG).await;
    for status in REASONED {
        let customer = db.customer(ORG).await;
        assert_check_violation(
            db.case(&customer, status, Some(reviewer), true, None)
                .await
                .map(|_| ()),
            &format!("{status} with no decision_reason_code"),
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn a_reason_is_one_of_the_six_codes_and_nothing_else() {
    // S-5, decided 2026-10-12 (PR 1438, verification.md §12; the lane controller's ruling on #1386): six codes,
    // no free text stored.
    let db = Db::fresh().await;
    let reviewer = db.user(ORG).await;
    for status in REASONED {
        for code in REASON_CODES {
            let customer = db.customer(ORG).await;
            db.case(&customer, status, Some(reviewer), true, Some(code))
                .await
                .unwrap_or_else(|e| panic!("{status} with reason {code}: {e}"));
        }
        for other in ["bad photo", "other", "", "R1", "DOCUMENT_UNREADABLE "] {
            let customer = db.customer(ORG).await;
            assert_check_violation(
                db.case(&customer, status, Some(reviewer), true, Some(other))
                    .await
                    .map(|_| ()),
                &format!("{status} with reason {other:?}, not one of the six codes"),
            );
        }
    }
    db.finish().await;
}

#[tokio::test]
async fn verified_without_reviewed_by_is_refused() {
    let db = Db::fresh().await;
    let reviewer = db.user(ORG).await;
    let customer = db.customer(ORG).await;
    assert_check_violation(
        db.case(&customer, "VERIFIED", None, true, None)
            .await
            .map(|_| ()),
        "VERIFIED with reviewed_by NULL",
    );
    db.case(&customer, "VERIFIED", Some(reviewer), true, None)
        .await
        .expect("VERIFIED with reviewed_by and decided_at");
    db.finish().await;
}

#[tokio::test]
async fn a_decided_case_needs_its_reviewer_and_decided_at() {
    // Design §4.1: CHECK (status NOT IN ('VERIFIED','REJECTED','NEEDS_UPDATE','REVOKED')
    //                     OR (reviewed_by IS NOT NULL AND decided_at IS NOT NULL)).
    let db = Db::fresh().await;
    let reviewer = db.user(ORG).await;
    for status in ["VERIFIED", "REJECTED", "NEEDS_UPDATE", "REVOKED"] {
        let reason = (status != "VERIFIED").then_some("OTHER");
        let customer = db.customer(ORG).await;
        assert_check_violation(
            db.case(&customer, status, Some(reviewer), false, reason)
                .await
                .map(|_| ()),
            &format!("{status} with reviewed_by but no decided_at"),
        );
        assert_check_violation(
            db.case(&customer, status, None, true, reason)
                .await
                .map(|_| ()),
            &format!("{status} with decided_at but no reviewed_by"),
        );
        db.case(&customer, status, Some(reviewer), true, reason)
            .await
            .unwrap_or_else(|e| panic!("{status} with reviewed_by and decided_at: {e}"));
    }
    db.finish().await;
}

#[tokio::test]
async fn m1_seeds_policy_version_1_for_every_existing_organisation() {
    let db = Db::empty().await;
    let m1 = db.m1();
    db.migrator
        .run_to(m1 - 1, &db.pool)
        .await
        .expect("migrate up to the one before M1");
    let second = db.organisation().await;
    let third = db.organisation().await;
    let before = db.count("SELECT count(*) FROM organizations").await;
    assert!(before >= 3, "ORG and two more organisations before M1");
    db.migrator
        .run_to(m1, &db.pool)
        .await
        .expect("M1 applies over existing organisations");
    assert_eq!(
        db.count("SELECT count(*) FROM verification_policies WHERE version = 1")
            .await,
        before,
        "§8 M1 item 2: count(verification_policies version 1) = count(organizations)"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM verification_policies").await,
        before,
        "exactly one row per organisation"
    );
    for org in [ORG.to_owned(), second.to_string(), third.to_string()] {
        let row: (i32, Vec<String>, i32, bool, bool) = sqlx::query_as(
            "SELECT version, accepted_document_types, documents_required, branch_review_allowed, \
             published_by IS NULL \
             FROM verification_policies WHERE organization_id = $1::uuid",
        )
        .bind(&org)
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("the policy row of {org}: {e}"));
        let (version, mut types, required, branch_review, published_by_null) = row;
        types.sort();
        assert_eq!(version, 1, "{org}: version 1");
        assert_eq!(
            types,
            vec!["NATIONAL_ID".to_owned(), "PASSPORT".to_owned()],
            "{org}: OQ-1 {{PASSPORT, NATIONAL_ID}}"
        );
        assert_eq!(required, 1, "{org}: OQ-1 documents_required 1");
        assert!(branch_review, "{org}: OQ-5 branch_review_allowed true");
        assert!(
            published_by_null,
            "{org}: version 1 seeded by M1, published_by NULL"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn an_organisation_created_after_m1_has_no_policy_row() {
    let db = Db::fresh().await;
    let late = db.organisation().await;
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM verification_policies WHERE organization_id = '{late}'::uuid"
        ))
        .await,
        0,
        "design §9 edge: no seed for a later organisation (VF-3 answers noVerificationPolicy)"
    );
    db.finish().await;
}

#[tokio::test]
async fn published_by_null_is_allowed_only_for_version_1() {
    let db = Db::fresh().await;
    let publisher = db.user(ORG).await;
    let insert = |version: i32, by: Option<Uuid>| {
        let by = by.map_or("NULL".to_owned(), |b| format!("'{b}'::uuid"));
        format!(
            "INSERT INTO verification_policies (organization_id, version, accepted_document_types, \
             documents_required, branch_review_allowed, published_by, published_at, created_at) \
             VALUES ('{ORG}'::uuid, {version}, ARRAY['PASSPORT'], 1, false, {by}, now(), now())"
        )
    };
    assert_check_violation(
        db.exec(&insert(2, None)).await,
        "policy version 2 with published_by NULL",
    );
    db.ok(&insert(2, Some(publisher))).await;
    db.finish().await;
}

#[tokio::test]
async fn provider_is_staff_only_and_external_case_ref_is_always_null() {
    let db = Db::fresh().await;
    let customer = db.customer(ORG).await;
    // EXPIRED carries none of the reviewer or reason checks, so only the column under test can refuse.
    let insert = |extra_cols: &str, extra_vals: &str| {
        format!(
            "INSERT INTO verification_cases (id, organization_id, customer_id, status, policy_version, \
             created_at{extra_cols}) VALUES ('{}', '{ORG}'::uuid, '{customer}', 'EXPIRED', 1, now(){extra_vals})",
            Uuid::new_v4()
        )
    };
    assert_check_violation(
        db.exec(&insert(", provider", ", 'RECOGNIZE'")).await,
        "provider 'RECOGNIZE'",
    );
    assert_check_violation(
        db.exec(&insert(", external_case_ref", ", 'EXT-1'")).await,
        "a non-NULL external_case_ref",
    );
    db.ok(&insert("", "")).await;
    assert_eq!(
        db.text(&format!(
            "SELECT provider FROM verification_cases WHERE customer_id = '{customer}'"
        ))
        .await,
        "STAFF",
        "V5 / OQ-3: provider defaults to STAFF"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_case_branch_must_belong_to_the_case_organisation() {
    let db = Db::fresh().await;
    let own = db.branch(ORG).await;
    let other_org = db.organisation().await;
    let foreign = db.branch(&other_org.to_string()).await;
    let customer = db.customer(ORG).await;
    let insert = |branch: &str| {
        format!(
            "INSERT INTO verification_cases (id, organization_id, customer_id, status, policy_version, branch_id, \
             created_at) VALUES ('{}', '{ORG}'::uuid, '{customer}', 'EXPIRED', 1, '{branch}', now())",
            Uuid::new_v4()
        )
    };
    let r = refusal(
        db.exec(&insert(&foreign)).await,
        "a case of ORG at another organisation's branch",
    );
    assert_eq!(
        r.code, "23503",
        "V17: FK (organization_id, branch_id): {}",
        r.message
    );
    db.ok(&insert(&own)).await;
    db.finish().await;
}

#[tokio::test]
async fn a_document_is_jpeg_png_or_pdf_of_1_to_15_mib() {
    let db = Db::fresh().await;
    let uploader = db.user(ORG).await;
    let customer = db.customer(ORG).await;
    let case = db.valid_case(&customer, "DRAFT", uploader).await;
    for size in [0, MAX_UPLOAD_BYTES + 1] {
        assert_check_violation(
            db.document(case, uploader, Doc::stored("image/jpeg", size))
                .await,
            &format!("size_bytes {size}"),
        );
    }
    for mime in ["image/gif", "text/plain", "image/webp"] {
        assert_check_violation(
            db.document(case, uploader, Doc::stored(mime, 10)).await,
            &format!("mime {mime}"),
        );
    }
    for (mime, size) in [
        ("image/jpeg", 1),
        ("image/png", MAX_UPLOAD_BYTES),
        ("application/pdf", 1024),
    ] {
        db.document(case, uploader, Doc::stored(mime, size))
            .await
            .unwrap_or_else(|e| panic!("{mime} of {size} bytes: {e}"));
    }
    db.finish().await;
}

#[tokio::test]
async fn the_object_ref_is_null_exactly_when_the_document_is_removed_or_purged() {
    let db = Db::fresh().await;
    let uploader = db.user(ORG).await;
    let customer = db.customer(ORG).await;
    let case = db.valid_case(&customer, "DRAFT", uploader).await;
    assert_check_violation(
        db.document(
            case,
            uploader,
            Doc {
                mime: "image/png",
                size: 10,
                object_ref: None,
                removed: false,
                purged: false,
            },
        )
        .await,
        "no object ref on a document neither removed nor purged",
    );
    assert_check_violation(
        db.document(
            case,
            uploader,
            Doc {
                mime: "image/png",
                size: 10,
                object_ref: Some(unique("obj")),
                removed: true,
                purged: false,
            },
        )
        .await,
        "an object ref on a removed document",
    );
    assert_check_violation(
        db.document(
            case,
            uploader,
            Doc {
                mime: "image/png",
                size: 10,
                object_ref: Some(unique("obj")),
                removed: false,
                purged: true,
            },
        )
        .await,
        "an object ref on a purged document",
    );
    db.document(
        case,
        uploader,
        Doc {
            mime: "image/png",
            size: 10,
            object_ref: None,
            removed: true,
            purged: false,
        },
    )
    .await
    .expect("a removed document with no object ref");
    db.document(
        case,
        uploader,
        Doc {
            mime: "image/png",
            size: 10,
            object_ref: None,
            removed: false,
            purged: true,
        },
    )
    .await
    .expect("a purged document with no object ref");
    let shared = unique("obj");
    db.document(
        case,
        uploader,
        Doc {
            mime: "image/png",
            size: 10,
            object_ref: Some(shared.clone()),
            removed: false,
            purged: false,
        },
    )
    .await
    .expect("a stored document");
    let r = refusal(
        db.document(
            case,
            uploader,
            Doc {
                mime: "image/png",
                size: 10,
                object_ref: Some(shared.clone()),
                removed: false,
                purged: false,
            },
        )
        .await,
        "a second document with the same object ref",
    );
    assert_eq!(
        r.code, "23505",
        "encrypted_object_ref is UNIQUE: {}",
        r.message
    );
    db.finish().await;
}

#[tokio::test]
async fn m1_leaves_files_transaction_files_identities_and_retention_unchanged() {
    let db = Db::empty().await;
    let m1 = db.m1();
    db.migrator
        .run_to(m1 - 1, &db.pool)
        .await
        .expect("migrate up to the one before M1");
    let branch = db.branch(ORG).await;
    let customer = db.customer(ORG).await;
    let key = unique("file");
    db.ok(&format!(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at, organization_id) \
         VALUES ('{key}', '{branch}', 'teller', 'image/jpeg', 'abc', now(), '{ORG}'::uuid)"
    ))
    .await;
    db.ok(&format!(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at, organization_id) \
         VALUES ('{}', '{branch}', 'teller', 'application/pdf', 'def', now(), '{ORG}'::uuid)",
        unique("file")
    ))
    .await;
    let tx = Uuid::new_v4();
    db.ok(&format!(
        "INSERT INTO transactions (id, branch_id, mode, submission_id, payload_hash, customer_id, customer_name, \
         nationality, type, code, amount, rate, principal, fee, bank_fee, received, paid, method, payment_ref, \
         account, staff_id, booking_no, notes, value_date, paid_at, recorded_at, recorded_by, organization_id, \
         status) \
         VALUES ('{tx}', '{branch}', 'DEMO', '{}', 'h', '{customer}', 'Somchai', 'THA', 'BUY', 'USD', 100, 35, \
         3500, 0, 0, 3500, 3500, 'CASH', '', '', 'teller', '', '', current_date, now(), now(), 'teller', \
         '{ORG}'::uuid, 'DRAFT')",
        unique("sub")
    ))
    .await;
    db.ok(&format!(
        "INSERT INTO transaction_files (transaction_id, kind, file_key, organization_id) \
         VALUES ('{tx}', 'PASSPORT', '{key}', '{ORG}'::uuid)"
    ))
    .await;
    let watched = [
        "files",
        "transaction_files",
        "customer_identities",
        "retention_policies",
    ];
    let mut before = Vec::new();
    for table in watched {
        before.push((db.shape(table).await, db.rows(table).await));
    }
    assert!(
        before[0].1.starts_with("2 ") && before[1].1.starts_with("1 "),
        "seeded two files and one PASSPORT attachment: {:?}",
        (&before[0].1, &before[1].1)
    );
    db.migrator
        .run_to(m1, &db.pool)
        .await
        .expect("M1 applies over existing files and attachments");
    for (table, (shape, rows)) in watched.iter().zip(before) {
        assert_eq!(
            db.shape(table).await,
            shape,
            "§6 / V15 / S-4: M1 changes no column or constraint of {table}"
        );
        assert_eq!(
            db.rows(table).await,
            rows,
            "§8 M1 item 1: M1 changes no row of {table} (count and content)"
        );
    }
    db.finish().await;
}
