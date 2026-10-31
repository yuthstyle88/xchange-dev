//! #5665 part 1: `adminCreateBranch` puts the new branch in the caller's organisation, not in the
//! seeded one the `branches.organization_id` column default names.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0; design
//! `docs/design/org-wide-actor-branch.md` §15.11). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! The caller is the owner's user moved into a second organisation, signed in through
//! `user_sessions` like every other caller of `/api/rpc`. Needs `TEST_DATABASE_URL` (a database
//! whose name ends in `_test`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::{Duration, Utc};
use rpc_common::{Answer, App, Call, Desk, one_of_each_role};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
/// The organisation the migrations seed; the `branches.organization_id` default.
const SEEDED_ORG: &str = "00000000-0000-0000-0000-000000000001";
/// A second organisation, made by this test.
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
/// Organisation B's own branch.
const B_HOME: &str = "BB1";

struct Room {
    desk: Desk,
    app: App,
    /// The owner's user, moved into organisation B.
    owner: String,
}

impl Room {
    /// The usual desk, plus organisation B with one branch, and the owner's user moved into B
    /// with its organisation-wide roles.
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        desk.employ(&one_of_each_role()).await;
        let pool = &desk.db.pool;

        let owner: String = sqlx::query_scalar(
            "SELECT u.id::text FROM users u \
             WHERE u.organization_id = $1::uuid \
               AND EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id \
                           AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
             ORDER BY u.created_at LIMIT 1",
        )
        .bind(SEEDED_ORG)
        .fetch_one(pool)
        .await
        .expect("the owner's user");

        sqlx::query(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ($1::uuid, 'XB', 'Asia/Bangkok', now())",
        )
        .bind(ORG_B)
        .execute(pool)
        .await
        .expect("insert organisation B");
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, code) VALUES ($1, $2, $3::uuid, $1)",
        )
        .bind(B_HOME)
        .bind("Organisation B home")
        .bind(ORG_B)
        .execute(pool)
        .await
        .expect("insert organisation B's branch");
        sqlx::query("INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0)")
            .bind(B_HOME)
            .execute(pool)
            .await
            .expect("insert its version row");

        // Only organisation-wide roles move; a role held at a branch of the seeded organisation
        // cannot follow the user into B.
        sqlx::query("DELETE FROM user_roles WHERE user_id = $1::uuid AND branch_id IS NOT NULL")
            .bind(&owner)
            .execute(pool)
            .await
            .expect("drop the owner's branch roles");
        sqlx::query("UPDATE user_roles SET organization_id = $2::uuid WHERE user_id = $1::uuid")
            .bind(&owner)
            .bind(ORG_B)
            .execute(pool)
            .await
            .expect("move the owner's roles into B");
        sqlx::query("UPDATE users SET organization_id = $2::uuid WHERE id = $1::uuid")
            .bind(&owner)
            .bind(ORG_B)
            .execute(pool)
            .await
            .expect("move the owner's user into B");

        Room { desk, app, owner }
    }

    /// The owner's session cookie, its effective branch `branch`.
    async fn sign_in(&self, branch: Option<&str>) -> String {
        let mut conn = self.desk.db.pool.acquire().await.expect("acquire");
        let token = xc_testdb::sessions::seed_session(
            &mut conn,
            &self.owner,
            branch,
            Utc::now() + Duration::hours(1),
        )
        .await
        .expect("sign the owner in");
        xc_testdb::sessions::cookie(&token)
    }

    async fn create_branch(&self, cookie: &str, request_id: &str, payload: Value) -> Answer {
        let mut payload = payload;
        payload["requestId"] = json!(request_id);
        let body = json!({ "action": "adminCreateBranch", "payload": payload }).to_string();
        self.app
            .send(Call::post(ORIGIN, "/api/rpc", body).cookie(cookie))
            .await
    }

    /// The organisation of the branch created with `request_id`, if one was.
    async fn organisation_of(&self, request_id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE request_id = $1")
            .bind(request_id)
            .fetch_optional(&self.desk.db.pool)
            .await
            .expect("read branches")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

#[actix_web::test]
async fn a_branch_created_from_organisation_bs_branch_belongs_to_organisation_b() {
    let room = Room::open().await;
    let cookie = room.sign_in(Some(B_HOME)).await;
    let request = "00000000-0000-4000-8000-0000000b2001";

    let answer = room
        .create_branch(&cookie, request, json!({ "name": "B Silom" }))
        .await;

    assert_eq!(answer.status, 200, "created: {:.300}", answer.text);
    assert_eq!(
        room.organisation_of(request).await.as_deref(),
        Some(ORG_B),
        "the new branch is organisation B's, not the column default's"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_branch_created_with_no_session_branch_belongs_to_the_callers_organisation() {
    let room = Room::open().await;
    let cookie = room.sign_in(None).await;
    let request = "00000000-0000-4000-8000-0000000b2002";

    let answer = room
        .create_branch(&cookie, request, json!({ "name": "B Sathorn" }))
        .await;

    assert_eq!(answer.status, 200, "created: {:.300}", answer.text);
    assert_eq!(
        room.organisation_of(request).await.as_deref(),
        Some(ORG_B),
        "an organisation-wide caller with no branch still creates in its own organisation"
    );
    room.finish().await;
}
