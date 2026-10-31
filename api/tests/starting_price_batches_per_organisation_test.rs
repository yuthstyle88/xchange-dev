//! #5666 D5: an organisation's starting-price batch carries that organisation, and the
//! reference base reads a branch's `START` line only from its own organisation's batches. Market
//! data stays shared (owner ruling D5 = A).
//!
//! Written before the implementation by the ticket author (claude/b091f5e0; design
//! `docs/design/org-wide-actor-branch.md` §15.17, owner ruling D5 = A relayed on PR #5740
//! comment 6536029938). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Organisation A is the seeded one; organisation B is made here, with one branch and its own
//! owner (an organisation-wide EXECUTIVE + ADMIN, as `Desk::owner_session` seeds for A). Needs
//! `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

#[macro_use]
mod auth_http_common;
mod common;

use auth_http_common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::draft::reference_base::ReferenceBase;

/// The organisation the migrations seed (A).
const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
/// A second organisation, made by this test.
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
/// Organisation B's own branch.
const B_HOME: &str = "BB1";

const SAVE: &str = "/api/v3/commands/org_starting_price_save";

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// Organisation B with one branch, and a session of B's owner.
async fn organisation_b_owner(desk: &Desk) -> String {
    let pool = &desk.pool;
    exec(
        pool,
        &format!(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ('{ORG_B}', 'XB', 'Asia/Bangkok', now())"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO branches (id, name, organization_id, code) \
             VALUES ('{B_HOME}', 'Organisation B home', '{ORG_B}', '{B_HOME}')"
        ),
    )
    .await;
    exec(
        pool,
        &format!("INSERT INTO branch_versions (branch_id, version) VALUES ('{B_HOME}', 0)"),
    )
    .await;

    let mut conn = pool.acquire().await.expect("a connection");
    let valid_from = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
    let id = xc_testdb::people::seed_person(
        &mut conn,
        ORG_B,
        "b-boss",
        &[
            xc_testdb::people::RoleRow {
                role_code: "EXECUTIVE",
                branch_id: None,
                valid_from,
                valid_to: None,
            },
            xc_testdb::people::RoleRow {
                role_code: "ADMIN",
                branch_id: None,
                valid_from,
                valid_to: None,
            },
        ],
    )
    .await
    .expect("seed organisation B's owner");
    xc_testdb::sessions::seed_session(
        &mut conn,
        &id,
        None,
        chrono::Utc::now() + chrono::Duration::hours(12),
    )
    .await
    .expect("seed B's owner session")
}

/// The reference line `ReferenceBase` picks for `branch` and USD.
async fn usd_base_of(pool: &PgPool, branch: &str) -> Option<Uuid> {
    let mut tx = pool.begin().await.expect("begin");
    let got = ReferenceBase::newest_in(
        &mut tx,
        &BranchId::new(branch),
        &CurrencyCode::parse("USD").expect("USD"),
    )
    .await
    .expect("newest_in answers");
    tx.rollback().await.expect("rollback");
    got
}

#[actix_web::test]
async fn organisation_bs_starting_price_batch_belongs_to_organisation_b() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let b_owner = organisation_b_owner(&desk).await;

    let answer = send(
        &app,
        Call::post(
            SAVE,
            json!({
                "prices": [{ "currency": "USD", "buy": "33.10", "sell": "33.40" }],
                "reason": "B opens",
            }),
        )
        .session(&b_owner)
        .header("idempotency-key", &format!("idem-{}", Uuid::new_v4())),
    )
    .await;
    assert_eq!(answer.status, 200, "saved: {}", answer.text);

    let organisations: Vec<String> = sqlx::query_scalar(
        "SELECT organization_id::text FROM rate_reference_batches WHERE provider = $1",
    )
    .bind(format!("START:{ORG_B}"))
    .fetch_all(&desk.pool)
    .await
    .expect("read rate_reference_batches");
    assert_eq!(
        organisations,
        vec![ORG_B.to_owned()],
        "B's starting-price batch is B's, not the column default's"
    );
}

#[actix_web::test]
async fn a_start_batch_whose_organisation_is_not_the_branchs_is_not_its_reference_base() {
    let desk = Desk::start().await;
    organisation_b_owner(&desk).await;
    // No market line for USD, so BB1's BOT anchor falls back to the START line.
    exec(
        &desk.pool,
        "DELETE FROM rate_reference_lines WHERE currency_code = 'USD' AND reference_type = 'BOT'",
    )
    .await;
    // A batch named for B whose organisation column is A's: the shape every START batch has on
    // main before D5, because the writer leaves the column to its default.
    exec(
        &desk.pool,
        &format!(
            "WITH b AS (INSERT INTO rate_reference_batches \
                 (id, provider, source_date, fetched_at, raw_checksum, status, organization_id) \
                 VALUES (gen_random_uuid(), 'START:{ORG_B}', current_date, now(), 'd5', 'COMPLETE', '{ORG_A}') \
                 RETURNING id) \
             INSERT INTO rate_reference_lines \
                 (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
             SELECT gen_random_uuid(), b.id, 'USD', 'START', 1, 33.10, 33.40, '{{}}'::jsonb FROM b"
        ),
    )
    .await;

    assert_eq!(
        usd_base_of(&desk.pool, B_HOME).await,
        None,
        "a START line counts for a branch only when its batch's organisation is the branch's"
    );
}
