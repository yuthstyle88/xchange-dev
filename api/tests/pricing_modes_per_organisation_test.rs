//! #5666 D1: `pricing_modes` is per organisation. One organisation's FOLLOW / MANUAL never changes
//! another's row, and each organisation's revision counts its own saves.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0; design
//! `docs/design/org-wide-actor-branch.md` §15.14, owner ruling D1 = A at #564
//! issuecomment-6520923886). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Organisation A is the seeded one; organisation B is made here, with its own owner (an
//! organisation-wide EXECUTIVE + ADMIN, as `Desk::owner_session` seeds for A). Both save through
//! `pricingModeSave` (`POST /api/pricing/mode`). Needs `TEST_DATABASE_URL` (a database whose name
//! ends in `_test`); nothing here skips.

#[macro_use]
mod auth_http_common;
mod common;

use auth_http_common::*;
use serde_json::json;
use sqlx::PgPool;

/// The organisation the migrations seed (A).
const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
/// A second organisation, made by this test.
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";

const MODE: &str = "/api/pricing/mode";

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
             VALUES ('BB1', 'Organisation B home', '{ORG_B}', 'BB1')"
        ),
    )
    .await;
    exec(
        pool,
        "INSERT INTO branch_versions (branch_id, version) VALUES ('BB1', 0)",
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

async fn save_mode(app: &TestApp, token: &str, currency: &str, mode: &str) {
    let answer = send(
        app,
        Call::post(MODE, json!({ "currency": currency, "mode": mode }))
            .session(token)
            .header("idempotency-key", &format!("idem-{}", uuid::Uuid::new_v4())),
    )
    .await;
    assert_eq!(answer.status, 200, "{currency} {mode}: {}", answer.text);
}

#[actix_web::test]
async fn organisation_bs_mode_leaves_organisation_as_mode_alone() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let a_owner = desk.owner_session("a-boss").await;
    let b_owner = organisation_b_owner(&desk).await;

    save_mode(&app, &a_owner, "USD", "FOLLOW").await;
    save_mode(&app, &b_owner, "USD", "MANUAL").await;

    let modes: Vec<String> = sqlx::query_scalar(
        "SELECT mode FROM pricing_modes WHERE trim(currency_code) = 'USD' ORDER BY mode",
    )
    .fetch_all(&desk.pool)
    .await
    .expect("read pricing_modes");
    assert_eq!(
        modes,
        vec!["FOLLOW".to_owned(), "MANUAL".to_owned()],
        "A's FOLLOW and B's MANUAL are two rows; B's save did not overwrite A's"
    );
}

#[actix_web::test]
async fn each_organisations_mode_row_is_its_own() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let a_owner = desk.owner_session("a-boss").await;
    let b_owner = organisation_b_owner(&desk).await;

    save_mode(&app, &a_owner, "USD", "FOLLOW").await;
    save_mode(&app, &b_owner, "USD", "MANUAL").await;
    save_mode(&app, &b_owner, "JPY", "FOLLOW").await;

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT organization_id::text, trim(currency_code), mode FROM pricing_modes \
         ORDER BY organization_id, currency_code",
    )
    .fetch_all(&desk.pool)
    .await
    .expect("read pricing_modes");
    assert_eq!(
        rows,
        vec![
            (ORG_A.to_owned(), "USD".to_owned(), "FOLLOW".to_owned()),
            (ORG_B.to_owned(), "JPY".to_owned(), "FOLLOW".to_owned()),
            (ORG_B.to_owned(), "USD".to_owned(), "MANUAL".to_owned()),
        ],
        "every row carries the organisation that saved it"
    );
}

#[actix_web::test]
async fn each_organisations_revision_counts_its_own_saves() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let a_owner = desk.owner_session("a-boss").await;
    let b_owner = organisation_b_owner(&desk).await;

    save_mode(&app, &a_owner, "USD", "FOLLOW").await;
    save_mode(&app, &a_owner, "USD", "MANUAL").await;
    save_mode(&app, &a_owner, "USD", "FOLLOW").await;
    save_mode(&app, &b_owner, "USD", "FOLLOW").await;

    let revisions: Vec<(String, i64)> = sqlx::query_as(
        "SELECT organization_id::text, revision FROM pricing_modes \
         WHERE trim(currency_code) = 'USD' ORDER BY organization_id",
    )
    .fetch_all(&desk.pool)
    .await
    .expect("read pricing_modes");
    assert_eq!(
        revisions,
        vec![(ORG_A.to_owned(), 3), (ORG_B.to_owned(), 1)],
        "B's first save is revision 1, whatever A has saved"
    );
}
