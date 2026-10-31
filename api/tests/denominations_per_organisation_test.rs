//! #5666 D2: denominations and their groups belong to one organisation. A group an organisation
//! creates or maps is its own; its views list its own rows only.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0; design
//! `docs/design/org-wide-actor-branch.md` §15.15, owner ruling D2 = A at #564
//! issuecomment-6520923886). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Organisation A is the seeded one; organisation B is made here, with its own owner (an
//! organisation-wide EXECUTIVE + ADMIN, as `Desk::owner_session` seeds for A). Needs
//! `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

#[macro_use]
mod auth_http_common;
mod common;

use actix_web::http::Method;
use auth_http_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// The organisation the migrations seed (A).
const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
/// A second organisation, made by this test.
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";

const CREATE: &str = "/api/denomination-groups";
const MAP: &str = "/api/denomination-groups/map";
const DENOMINATIONS: &str = "/api/v3/views/denominations";
const ADMIN_GROUPS: &str = "/api/v3/views/admin_denomination_groups";

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

async fn post(app: &TestApp, token: &str, path: &str, body: Value) -> Answer {
    send(
        app,
        Call::post(path, body)
            .session(token)
            .header("idempotency-key", &format!("idem-{}", uuid::Uuid::new_v4())),
    )
    .await
}

async fn view(app: &TestApp, token: &str, path: &str) -> Value {
    let answer = send(
        app,
        Call {
            method: Some(Method::GET),
            path: path.to_owned(),
            proto: Some("https".to_owned()),
            ..Call::default()
        }
        .session(token),
    )
    .await;
    assert_eq!(answer.status, 200, "{path}: {}", answer.text);
    answer.json()["data"].clone()
}

async fn organisations_of_group(pool: &PgPool, currency: &str, group: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT organization_id::text FROM denominations \
         WHERE currency_code = $1 AND group_code = $2 ORDER BY organization_id",
    )
    .bind(currency)
    .bind(group)
    .fetch_all(pool)
    .await
    .expect("read denominations")
}

#[actix_web::test]
async fn a_group_organisation_b_creates_is_organisation_bs() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let b_owner = organisation_b_owner(&desk).await;

    let answer = post(
        &app,
        &b_owner,
        CREATE,
        json!({ "currency": "USD", "group": "BLARGE" }),
    )
    .await;

    assert_eq!(answer.status, 200, "created: {}", answer.text);
    assert_eq!(
        organisations_of_group(&desk.pool, "USD", "BLARGE").await,
        vec![ORG_B.to_owned()],
        "the new group row is B's, not the column default's"
    );
}

#[actix_web::test]
async fn organisation_b_may_create_a_group_organisation_a_already_has() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    exec(
        &desk.pool,
        "INSERT INTO denominations (currency_code, face_value, group_code) VALUES ('USD', NULL, 'LARGE')",
    )
    .await;
    let b_owner = organisation_b_owner(&desk).await;

    let answer = post(
        &app,
        &b_owner,
        CREATE,
        json!({ "currency": "USD", "group": "LARGE" }),
    )
    .await;

    assert_eq!(answer.status, 200, "created: {}", answer.text);
    assert_eq!(
        organisations_of_group(&desk.pool, "USD", "LARGE").await,
        vec![ORG_A.to_owned(), ORG_B.to_owned()],
        "A's LARGE does not make B's create a no-op"
    );
}

#[actix_web::test]
async fn organisation_bs_mapping_leaves_organisation_as_mapping_alone() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let a_owner = desk.owner_session("a-boss").await;
    let b_owner = organisation_b_owner(&desk).await;

    for (token, key) in [(&a_owner, "USD-A"), (&b_owner, "USD-B")] {
        let created = post(
            &app,
            token,
            CREATE,
            json!({ "currency": "USD", "group": "LARGE" }),
        )
        .await;
        assert_eq!(created.status, 200, "created: {}", created.text);
        let mapped = post(
            &app,
            token,
            MAP,
            json!({ "currency": "USD", "group": "LARGE", "provider": "RATE_BOARD", "instrumentKey": key }),
        )
        .await;
        assert_eq!(mapped.status, 200, "mapped {key}: {}", mapped.text);
    }

    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT instrument_key FROM denomination_group_sources \
         WHERE currency_code = 'USD' AND group_code = 'LARGE' AND provider = 'RATE_BOARD' \
         ORDER BY instrument_key",
    )
    .fetch_all(&desk.pool)
    .await
    .expect("read denomination_group_sources");
    assert_eq!(
        keys,
        vec!["USD-A".to_owned(), "USD-B".to_owned()],
        "one mapping per organisation; B's did not overwrite A's"
    );
}

#[actix_web::test]
async fn organisation_bs_denominations_view_lists_only_bs_face_values() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let b_owner = organisation_b_owner(&desk).await;
    exec(
        &desk.pool,
        &format!(
            "INSERT INTO denominations (currency_code, face_value, group_code, organization_id) VALUES \
               ('USD', 100, 'ALL', '{ORG_A}'), ('USD', 50, 'ALL', '{ORG_B}')"
        ),
    )
    .await;

    let data = view(&app, &b_owner, DENOMINATIONS).await;
    let rows: Vec<(String, String)> = data["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| {
            (
                r["currency"].as_str().expect("currency").to_owned(),
                r["faceValue"].as_str().expect("faceValue").to_owned(),
            )
        })
        .collect();

    assert_eq!(
        rows,
        vec![("USD".to_owned(), "50.0000".to_owned())],
        "B sees its own face value only; the seeded organisation's are A's"
    );
}

#[actix_web::test]
async fn organisation_bs_admin_groups_view_lists_only_bs_groups() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let b_owner = organisation_b_owner(&desk).await;
    let created = post(
        &app,
        &b_owner,
        CREATE,
        json!({ "currency": "USD", "group": "BLARGE" }),
    )
    .await;
    assert_eq!(created.status, 200, "created: {}", created.text);

    let data = view(&app, &b_owner, ADMIN_GROUPS).await;

    assert_eq!(
        data["currencies"],
        json!([{
            "currency": "USD",
            "groups": [{ "group": "BLARGE", "instrumentKey": null, "removable": true }]
        }]),
        "B lists its own group only; the seeded organisation's ALL rows are A's"
    );
}
