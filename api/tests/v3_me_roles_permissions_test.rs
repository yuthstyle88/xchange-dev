//! Ticket #1030 (OR-7.7): the v3 seams hand implementors `access::Actor`, and the `me` view answers `roles` and
//! `permissions` instead of the old `kind`, `role` and `username` (the owner's decision, 2026-10-10: "roles +
//! permissions"). Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/org-roles-branches.md` §3 (the roles and §3.3's mapping) and §14 (SB-1's `Me`
//! branch fields). Ticket §3.3–§3.9 and §8–§9.
//!
//! Shapes these tests assume (ticket §7, [required] JSON fields):
//! - `GET /api/v3/views/me` → `data` = `{id, name, roles, permissions, branch, branchName, branches}`, and nothing
//!   else: no `kind`, `role` or `username`.
//!   - `id` = the session's `users.id`; `name` = `users.name`, or `null` when the caller has no `users` row.
//!   - `roles` = `[{role, branchId}]`: the caller's valid-now rows, `role` the wire name (`"TELLER"`), `branchId` a
//!     branch id or `null`, ordered by role name, then branch id.
//!   - `permissions` = the `Permission` variant names (serde form, e.g. `"ApprovalsDecide"`) that
//!     `Authorizer::check` grants at `Me.branch`, the candidates being the union of `RoleGrants::seed` over the
//!     caller's roles; sorted, no duplicates. `Authorizer::check` is the only judge, so these tests use it as the
//!     oracle rather than copy a scope rule.
//! - The `me` version moves when a role row is granted or revoked (`identity_version` over `user_id` and the sorted
//!   `role@branch` list).
//! - `GET /api/v3/views/admin_branches` → `data.branches` lists exactly the branches the caller may switch to
//!   (SB-1's `SessionBranch::switchable`).
//! - `PUT /api/branches/profile` (`branchSave`, #945) on another organisation's branch answers `outOfScope` (#945
//!   ruling 1: the seam now passes the actor's organisation).
//! - `POST /api/rate-policies` (`ratePolicySave`): the owner-only fields (the guards and the stale settings) may be
//!   changed iff the caller has a valid-now EXECUTIVE row covering the branch; RATE_MANAGER keeps today's limited
//!   edit, refused as `PolicyError::RoleNotAllowedHere`.
//!
//! Callers are real Identity sessions (SB-1's harness), so a caller can hold exactly the rows a test needs, which
//! the old owner/staff cookies cannot express (an old owner is always EXECUTIVE and ADMIN together).
//!
//! `TEST_DATABASE_URL` must name a database whose name ends in `_test`; each test gets its own schema.

mod auth_http_common;
mod common;
mod identity_signin_common;

use std::collections::BTreeSet;

use actix_web::http::Method;
use auth_http_common::{Call, Desk, TestApp, owner_signs_in, send};
use chrono::{TimeZone, Utc};
use identity_signin_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::authorizer::{Authorizer, Target};
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::{Group, RoleGrants, RoleGrantsSnapshot};
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::error::V3Error;

// ── setup ─────────────────────────────────────────────────────────────────

/// Three branches beside `MAIN`. Codes sort A01 (B1) < A03 (B3) < MAIN < Z02 (B2), so SB-1's R5 starts a user
/// holding B1 on B1.
async fn branches(pool: &PgPool) {
    let org = organization(pool).await;
    for (id, code, name) in [
        ("B1", "A01", "สาขาหนึ่ง"),
        ("B2", "Z02", "สาขาสอง"),
        ("B3", "A03", "สาขาสาม"),
    ] {
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, code) VALUES ($1, $2, $3::uuid, $4)",
        )
        .bind(id)
        .bind(name)
        .bind(&org)
        .bind(code)
        .execute(pool)
        .await
        .expect("insert a branch");
    }
}

async fn setup() -> (Desk, FakeIdentity, App, TestApp) {
    let desk = Desk::start().await;
    branches(&desk.pool).await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let reader = desk_app_without_custody!(desk);
    (desk, identity, app, reader)
}

/// Signs in as the user bound to the subject of `token`; the session cookie value.
async fn sign_in(app: &App, identity: &FakeIdentity, token: &str) -> String {
    identity.login_answers("/auth/login", token, REFRESH_1);
    let reply = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "+66812345678", "password": "pw-from-the-browser-01" }),
        ))
        .await;
    assert_eq!(reply.status, 200, "sign-in: {}", reply.text);
    reply.session()
}

async fn switch_to(app: &App, token: &str, branch: &str) {
    let reply = app
        .send(
            Req::post("/api/auth/session/branch", json!({ "branch": branch })).with_session(token),
        )
        .await;
    assert_eq!(reply.status, 200, "switch to {branch}: {}", reply.text);
}

fn session_cookie(token: &str) -> String {
    format!("{SESSION_COOKIE}={token}")
}

/// A GET of `path` with the session cookie; the whole body of a 200.
async fn read(reader: &TestApp, token: &str, path: &str) -> Value {
    let answer = send(
        reader,
        Call {
            method: Some(Method::GET),
            path: path.to_owned(),
            proto: Some("https".to_owned()),
            cookie: Some(session_cookie(token)),
            ..Call::default()
        },
    )
    .await;
    assert_eq!(answer.status, 200, "GET {path}: {}", answer.text);
    answer.json()
}

async fn me(reader: &TestApp, token: &str) -> Value {
    read(reader, token, "/api/v3/views/me").await["data"].clone()
}

/// Ends the caller's `role` row at `branch` one second ago, as a revocation does.
async fn end_row(pool: &PgPool, user: &str, role: &str, branch: Option<&str>) {
    let done = sqlx::query(
        "UPDATE user_roles SET valid_to = now() - interval '1 second' \
         WHERE user_id = $1::uuid AND role_code = $2 AND branch_id IS NOT DISTINCT FROM $3",
    )
    .bind(user)
    .bind(role)
    .bind(branch)
    .execute(pool)
    .await
    .expect("end a role row");
    assert_eq!(
        done.rows_affected(),
        1,
        "the {role} row at {branch:?} exists"
    );
}

/// Grants `role` at `branch` from now on.
async fn grant_row(pool: &PgPool, user: &str, role: &str, branch: Option<&str>) {
    let org = organization(pool).await;
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
         VALUES ($1::uuid, $2, $3::uuid, $4, now() - interval '1 second')",
    )
    .bind(user)
    .bind(role)
    .bind(&org)
    .bind(branch)
    .execute(pool)
    .await
    .expect("grant a role row");
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("an object: {v}"))
        .keys()
        .cloned()
        .collect()
}

fn me_keys() -> BTreeSet<String> {
    [
        "id",
        "name",
        "roles",
        "permissions",
        "branch",
        "branchName",
        "branches",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("an array: {v}"))
        .iter()
        .map(|s| {
            s.as_str()
                .unwrap_or_else(|| panic!("a string: {s}"))
                .to_owned()
        })
        .collect()
}

// ── the oracle: what `Authorizer::check` grants ───────────────────────────

fn role_of(wire: &str) -> Role {
    Role::parse(wire).unwrap_or_else(|| panic!("a known role: {wire}"))
}

/// The serde name of a permission, e.g. `"ApprovalsDecide"`.
fn name_of(p: Permission) -> String {
    serde_json::to_value(p)
        .expect("a permission serialises")
        .as_str()
        .expect("a permission serialises as a string")
        .to_owned()
}

/// The sorted permission names `Authorizer::check` grants at `branch` to an actor holding exactly `rows`, all valid
/// now. The candidates are the union of `RoleGrants::seed` over the rows (ticket §3.3).
fn granted(rows: &[(&str, Option<&str>)], branch: &str) -> Vec<String> {
    let org = "org-under-test";
    let since = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let actor = Actor::new(
        "user-under-test",
        org,
        rows.iter()
            .map(|(role, at)| ScopedRole::new(role_of(role), at.map(BranchId::new), since, None))
            .collect(),
        None,
    );
    let target = Target::new(org, Some(BranchId::new(branch)), None);
    let mut names: BTreeSet<String> = BTreeSet::new();
    for (role, _) in rows {
        for p in RoleGrants::seed(role_of(role)) {
            if Authorizer::check(Some(&actor), p, &target, Utc::now()).is_ok() {
                names.insert(name_of(p));
            }
        }
    }
    names.into_iter().collect()
}

// ── me: roles and permissions (§3.3, §8) ──────────────────────────────────

/// §8: an organisation-wide EXECUTIVE + ADMIN (the former owner).
#[actix_web::test]
async fn an_executive_and_admin_reads_both_rows_and_their_permissions_and_no_old_fields() {
    let (desk, identity, app, reader) = setup().await;
    let rows = [("EXECUTIVE", None), ("ADMIN", None)];
    let user = bound_user(&desk.pool, SUB_A, "ACTIVE", "Somchai", &rows).await;
    let token = sign_in(&app, &identity, VALID_A).await;
    switch_to(&app, &token, "B1").await;

    let data = me(&reader, &token).await;
    assert_eq!(keys(&data), me_keys(), "exactly the new fields: {data}");
    assert_eq!(data["id"], json!(user), "id is users.id");
    assert_eq!(data["name"], json!("Somchai"), "name is users.name");
    assert_eq!(data["branch"], json!("B1"));
    assert_eq!(
        data["roles"],
        json!([
            { "role": "ADMIN", "branchId": null },
            { "role": "EXECUTIVE", "branchId": null },
        ])
    );
    let permissions = strings(&data["permissions"]);
    assert_eq!(
        permissions,
        granted(&rows, "B1"),
        "what Authorizer::check grants at B1"
    );
    assert!(
        permissions.contains(&"ApprovalsDecide".to_owned()),
        "{permissions:?}"
    );
    drop(reader);
    desk.drop_schema().await;
}

/// §8: a TELLER at one branch.
#[actix_web::test]
async fn a_teller_reads_its_row_and_may_queue_and_exchange_but_not_decide_approvals() {
    let (desk, identity, app, reader) = setup().await;
    let rows = [("TELLER", Some("B1"))];
    bound_user(&desk.pool, SUB_A, "ACTIVE", "Nok", &rows).await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let data = me(&reader, &token).await;
    assert_eq!(keys(&data), me_keys(), "{data}");
    assert_eq!(data["branch"], json!("B1"));
    assert_eq!(
        data["roles"],
        json!([{ "role": "TELLER", "branchId": "B1" }])
    );
    let permissions = strings(&data["permissions"]);
    assert_eq!(permissions, granted(&rows, "B1"));
    for held in ["QueueOperate", "ExchangeTransact"] {
        assert!(
            permissions.contains(&held.to_owned()),
            "{held}: {permissions:?}"
        );
    }
    assert!(
        !permissions.contains(&"ApprovalsDecide".to_owned()),
        "{permissions:?}"
    );
    drop(reader);
    desk.drop_schema().await;
}

/// §9: a role granted at another branch lists its row, but grants nothing at the session branch.
#[actix_web::test]
async fn a_row_at_another_branch_is_listed_but_grants_nothing_here() {
    let (desk, identity, app, reader) = setup().await;
    let rows = [("TELLER", Some("B1")), ("FINANCE", Some("B2"))];
    bound_user(&desk.pool, SUB_A, "ACTIVE", "Mai", &rows).await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let at_b1 = me(&reader, &token).await;
    assert_eq!(at_b1["branch"], json!("B1"), "{at_b1}");
    assert_eq!(
        at_b1["roles"],
        json!([
            { "role": "FINANCE", "branchId": "B2" },
            { "role": "TELLER", "branchId": "B1" },
        ])
    );
    let here = strings(&at_b1["permissions"]);
    assert_eq!(here, granted(&rows, "B1"));
    assert!(
        !here.contains(&"FinanceReconcile".to_owned()),
        "FINANCE is at B2: {here:?}"
    );

    switch_to(&app, &token, "B2").await;
    let at_b2 = me(&reader, &token).await;
    assert_eq!(
        at_b2["roles"], at_b1["roles"],
        "the rows do not depend on the branch"
    );
    let there = strings(&at_b2["permissions"]);
    assert_eq!(there, granted(&rows, "B2"));
    assert!(there.contains(&"FinanceReconcile".to_owned()), "{there:?}");
    assert!(
        !there.contains(&"QueueOperate".to_owned()),
        "TELLER is at B1: {there:?}"
    );
    drop(reader);
    desk.drop_schema().await;
}

/// §8: a row whose `valid_to` has passed contributes nothing, neither a row nor a permission.
#[actix_web::test]
async fn an_ended_row_is_neither_listed_nor_granted() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Mai",
        &[("TELLER", Some("B1")), ("FINANCE", Some("B1"))],
    )
    .await;
    let token = sign_in(&app, &identity, VALID_A).await;
    end_row(&desk.pool, &user, "FINANCE", Some("B1")).await;

    let data = me(&reader, &token).await;
    assert_eq!(
        data["roles"],
        json!([{ "role": "TELLER", "branchId": "B1" }]),
        "{data}"
    );
    let permissions = strings(&data["permissions"]);
    assert_eq!(permissions, granted(&[("TELLER", Some("B1"))], "B1"));
    assert!(
        !permissions.contains(&"FinanceReconcile".to_owned()),
        "{permissions:?}"
    );
    drop(reader);
    desk.drop_schema().await;
}

/// §3.3: `roles` is ordered by role name, then branch id, whatever order the rows were granted in.
#[actix_web::test]
async fn roles_are_ordered_by_role_name_then_branch() {
    let (desk, identity, app, reader) = setup().await;
    bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Mai",
        &[
            ("TELLER", Some("B3")),
            ("RATE_MANAGER", Some("B1")),
            ("TELLER", Some("B1")),
            ("FINANCE", Some("B3")),
        ],
    )
    .await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let data = me(&reader, &token).await;
    assert_eq!(
        data["roles"],
        json!([
            { "role": "FINANCE", "branchId": "B3" },
            { "role": "RATE_MANAGER", "branchId": "B1" },
            { "role": "TELLER", "branchId": "B1" },
            { "role": "TELLER", "branchId": "B3" },
        ])
    );
    let permissions = strings(&data["permissions"]);
    let mut sorted = permissions.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(permissions, sorted, "sorted, no duplicates");
    drop(reader);
    desk.drop_schema().await;
}

// ── me: name and version (§3.3, §9) ───────────────────────────────────────

/// §9: `name` is `users.name` and follows a rename; nothing reads `owners` or `employees`.
#[actix_web::test]
async fn the_name_is_the_users_row_and_follows_a_rename() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Mai",
        &[("TELLER", Some("B1"))],
    )
    .await;
    let token = sign_in(&app, &identity, VALID_A).await;
    assert_eq!(me(&reader, &token).await["name"], json!("Mai"));

    sqlx::query("UPDATE users SET name = 'Mai Jaidee' WHERE id = $1::uuid")
        .bind(&user)
        .execute(&desk.pool)
        .await
        .expect("rename the user");
    assert_eq!(me(&reader, &token).await["name"], json!("Mai Jaidee"));
    drop(reader);
    desk.drop_schema().await;
}

/// §9: a role granted, or revoked, while the page polls `me` moves its version, as an account switch does.
#[actix_web::test]
async fn granting_or_revoking_a_role_moves_the_me_version() {
    let (desk, identity, app, reader) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Mai",
        &[("TELLER", Some("B1"))],
    )
    .await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let first = read(&reader, &token, "/api/v3/views/me").await;
    let v1 = first["version"].as_i64().expect("an integer version");
    let same = read(&reader, &token, &format!("/api/v3/views/me?since={v1}")).await;
    assert_eq!(same["changed"], json!(false), "nothing changed: {same}");

    grant_row(&desk.pool, &user, "FINANCE", Some("B1")).await;
    let granted = read(&reader, &token, &format!("/api/v3/views/me?since={v1}")).await;
    assert_eq!(
        granted["changed"],
        json!(true),
        "a grant moves the version: {granted}"
    );
    let v2 = granted["version"].as_i64().expect("an integer version");
    assert_ne!(v2, v1);
    assert_eq!(
        granted["data"]["roles"],
        json!([
            { "role": "FINANCE", "branchId": "B1" },
            { "role": "TELLER", "branchId": "B1" },
        ])
    );

    end_row(&desk.pool, &user, "FINANCE", Some("B1")).await;
    let revoked = read(&reader, &token, &format!("/api/v3/views/me?since={v2}")).await;
    assert_eq!(
        revoked["changed"],
        json!(true),
        "a revocation moves the version: {revoked}"
    );
    drop(reader);
    desk.drop_schema().await;
}

/// The old password cookies still reach `me` until OR-7.e: the owner's actor (`actor_from_v3`, EXECUTIVE + ADMIN,
/// id `"owner"`) has no `users` row, so `name` is `null`, and the read is a 200, not a failed id lookup.
#[actix_web::test]
async fn an_old_owner_cookie_still_reads_me_with_no_name() {
    let (desk, _identity, _app, reader) = setup().await;
    let owner = owner_signs_in(&reader).await;

    let answer = send(
        &reader,
        Call {
            method: Some(Method::GET),
            path: "/api/v3/views/me".to_owned(),
            proto: Some("https".to_owned()),
            ..Call::default()
        }
        .owner(&owner),
    )
    .await;
    assert_eq!(
        answer.status, 200,
        "me with the owner cookie: {}",
        answer.text
    );
    let data = answer.json()["data"].clone();
    assert_eq!(keys(&data), me_keys(), "{data}");
    assert_eq!(data["id"], json!("owner"));
    assert_eq!(data["name"], Value::Null);
    assert_eq!(
        data["roles"],
        json!([
            { "role": "ADMIN", "branchId": null },
            { "role": "EXECUTIVE", "branchId": null },
        ])
    );
    drop(reader);
    desk.drop_schema().await;
}

// ── admin_branches (§3.6) ───────────────────────────────────────────────

/// §9: `admin_branches` lists the branches the caller may switch to, not one branch and not all.
#[actix_web::test]
async fn admin_branches_lists_the_branches_the_caller_may_switch_to() {
    let (desk, identity, app, reader) = setup().await;
    bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("B1")), ("TELLER", Some("B3"))],
    )
    .await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let data = read(&reader, &token, "/api/v3/views/admin_branches").await["data"].clone();
    let listed: BTreeSet<String> = data["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("branches is an array: {data}"))
        .iter()
        .map(|b| b["id"].as_str().expect("an id").to_owned())
        .collect();
    let want: BTreeSet<String> = ["B1", "B3"].into_iter().map(str::to_owned).collect();
    assert_eq!(listed, want, "{data}");
    drop(reader);
    desk.drop_schema().await;
}

// ── ratePolicySave: who may move the frame (§3.9) ─────────────────────────

/// The guards of the first frame, as `v3_rate_policy_save_test.rs` enters it.
fn frame_guards() -> Value {
    json!({
        "USD": {
            "buy": { "min_pct": "-1.0000", "max_pct": "1.0000" },
            "sell": { "min_pct": "-1.0000", "max_pct": "1.0000" }
        },
        "JPY": {
            "buy": { "min_pct": "-2.0000", "max_pct": "0.5000" },
            "sell": { "min_pct": "-0.5000", "max_pct": "2.0000" }
        }
    })
}

fn frame(branch: &str, guards: Value, reason: &str) -> Value {
    json!({
        "branch": branch,
        "buyAdjustPct": "0",
        "sellAdjustPct": "0",
        "guards": guards,
        "maxReferenceAgeHours": 24,
        "onStale": "LIMIT",
        "staleLimitThb": "5000.00",
        "reason": reason,
    })
}

fn wider_guards() -> Value {
    let mut g = frame_guards();
    g["USD"]["buy"]["max_pct"] = json!("3.0000");
    g
}

async fn save_policy(
    reader: &TestApp,
    token: &str,
    body: Value,
    key: &str,
) -> auth_http_common::Answer {
    send(
        reader,
        Call {
            cookie: Some(session_cookie(token)),
            ..Call::post("/api/rate-policies", body)
        }
        .header("idempotency-key", key),
    )
    .await
}

async fn policy_versions(pool: &PgPool, branch: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM rate_policy_versions WHERE branch_id = $1")
        .bind(branch)
        .fetch_one(pool)
        .await
        .expect("count the policy versions")
}

fn assert_refused_as_outside_the_role(a: &auth_http_common::Answer, what: &str) {
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(
        a.status,
        err.status(),
        "{what}: the status of RoleNotAllowedHere; body {}",
        a.text
    );
    assert_eq!(
        a.json()["error"]["code"],
        json!(V3Error::from(err).code),
        "{what}: refused as a role outside its row; body {}",
        a.text
    );
}

/// §9: an EXECUTIVE (here without ADMIN, so not the old owner) may enter and widen the frame.
#[actix_web::test]
async fn an_executive_may_change_the_guards() {
    let (desk, identity, app, reader) = setup().await;
    bound_user(&desk.pool, SUB_A, "ACTIVE", "Exec", &[("EXECUTIVE", None)]).await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let first = save_policy(
        &reader,
        &token,
        frame("B1", frame_guards(), "first frame"),
        "rp-e-1",
    )
    .await;
    assert_eq!(
        first.status, 200,
        "EXECUTIVE enters the frame: {}",
        first.text
    );
    let wider = save_policy(
        &reader,
        &token,
        frame("B1", wider_guards(), "wider"),
        "rp-e-2",
    )
    .await;
    assert_eq!(
        wider.status, 200,
        "EXECUTIVE widens a guard: {}",
        wider.text
    );
    assert_eq!(policy_versions(&desk.pool, "B1").await, 2);
    drop(reader);
    desk.drop_schema().await;
}

/// §9: a RATE_MANAGER who moves a guard is refused exactly as today, and nothing is written.
#[actix_web::test]
async fn a_rate_manager_still_cannot_move_a_guard() {
    let (desk, identity, app, reader) = setup().await;
    // The frame is entered by the old owner's rows (EXECUTIVE + ADMIN), which reach the command today, so this
    // test pins today's refusal on the unchanged code as well as after the change.
    bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Owner",
        &[("EXECUTIVE", None), ("ADMIN", None)],
    )
    .await;
    bound_user(
        &desk.pool,
        SUB_B,
        "ACTIVE",
        "Rates",
        &[("RATE_MANAGER", Some("B1"))],
    )
    .await;
    let exec = sign_in(&app, &identity, VALID_A).await;
    let first = save_policy(
        &reader,
        &exec,
        frame("B1", frame_guards(), "first frame"),
        "rp-r-1",
    )
    .await;
    assert_eq!(first.status, 200, "the frame: {}", first.text);

    let rates = sign_in(&app, &identity, VALID_B).await;
    let moved = save_policy(
        &reader,
        &rates,
        frame("B1", wider_guards(), "wider"),
        "rp-r-2",
    )
    .await;
    assert_refused_as_outside_the_role(&moved, "RATE_MANAGER widening a guard");
    assert_eq!(
        policy_versions(&desk.pool, "B1").await,
        1,
        "nothing written"
    );
    drop(reader);
    desk.drop_schema().await;
}

/// §3.9: "an EXECUTIVE row" means a valid-now one. An ended EXECUTIVE row beside a RATE_MANAGER row leaves only the
/// RATE_MANAGER's limited edit.
#[actix_web::test]
async fn an_ended_executive_row_does_not_open_the_frame() {
    let (desk, identity, app, reader) = setup().await;
    // The frame is entered by the old owner's rows (EXECUTIVE + ADMIN), which reach the command today, so this
    // test pins today's refusal on the unchanged code as well as after the change.
    bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Owner",
        &[("EXECUTIVE", None), ("ADMIN", None)],
    )
    .await;
    let both = bound_user(
        &desk.pool,
        SUB_B,
        "ACTIVE",
        "Was exec",
        &[("EXECUTIVE", None), ("RATE_MANAGER", Some("B1"))],
    )
    .await;
    let exec = sign_in(&app, &identity, VALID_A).await;
    let first = save_policy(
        &reader,
        &exec,
        frame("B1", frame_guards(), "first frame"),
        "rp-x-1",
    )
    .await;
    assert_eq!(first.status, 200, "the frame: {}", first.text);

    end_row(&desk.pool, &both, "EXECUTIVE", None).await;
    let was = sign_in(&app, &identity, VALID_B).await;
    let moved = save_policy(
        &reader,
        &was,
        frame("B1", wider_guards(), "wider"),
        "rp-x-2",
    )
    .await;
    assert_refused_as_outside_the_role(&moved, "an ended EXECUTIVE row with a RATE_MANAGER row");
    assert_eq!(
        policy_versions(&desk.pool, "B1").await,
        1,
        "nothing written"
    );
    drop(reader);
    desk.drop_schema().await;
}

// ── branchSave across organisations (#945 ruling 1) ───────────────────────

/// The FE lane controller's [required] addition (#1030, from #945's ruling 1): once the command seam hands `run()`
/// the Blueprint actor with its organisation, `branchSave` (`PUT /api/branches/profile`, #945) on a branch of
/// **another** organisation is refused `outOfScope`, and that branch is unchanged. The same ADMIN saving its own
/// organisation's branch is the control: the refusal is about the organisation, not the caller.
#[actix_web::test]
async fn branch_save_on_another_organisations_branch_is_out_of_scope() {
    let (desk, identity, app, reader) = setup().await;
    bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin", &[("ADMIN", None)]).await;
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('00000000-0000-0000-0000-0000000000b2', 'YY', NULL, 'Asia/Bangkok', now())",
    )
    .execute(&desk.pool)
    .await
    .expect("another organisation");
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id, code) \
         VALUES ('X1', 'อื่น', '00000000-0000-0000-0000-0000000000b2', 'X1')",
    )
    .execute(&desk.pool)
    .await
    .expect("a branch of the other organisation");
    let token = sign_in(&app, &identity, VALID_A).await;

    let save = |branch: &str, code: &str, key: &str| {
        let mut call = Call {
            cookie: Some(session_cookie(&token)),
            ..Call::post(
                "/api/branches/profile",
                json!({
                    "branch": branch,
                    "code": code,
                    "address": "1 Silom Rd",
                    "timezone": "Asia/Bangkok",
                    "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
                    "bookingEnabled": true
                }),
            )
        }
        .header("idempotency-key", key);
        call.method = Some(Method::PUT);
        call
    };

    let own = send(&reader, save("B1", "SILOM", "bs-own-1")).await;
    assert_eq!(
        own.status, 200,
        "the ADMIN saves its own organisation's B1: {}",
        own.text
    );

    let other = send(&reader, save("X1", "TAKEN", "bs-other-1")).await;
    assert_eq!(
        other.json()["error"]["code"],
        json!("outOfScope"),
        "another organisation's branch: {}",
        other.text
    );
    let code: String = sqlx::query_scalar("SELECT code FROM branches WHERE id = 'X1'")
        .fetch_one(&desk.pool)
        .await
        .expect("X1's code");
    assert_eq!(code, "X1", "X1 is unchanged");
    drop(reader);
    desk.drop_schema().await;
}

// ── the generated type (§7) ───────────────────────────────────────────────

#[test]
fn the_generated_me_type_has_roles_and_permissions_and_no_old_fields() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/shared/api/generated/common.ts");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let me = text
        .lines()
        .find(|l| l.starts_with("export type Me = "))
        .unwrap_or_else(|| panic!("an `export type Me` line in {}", path.display()));
    for kept in [
        " id: string,",
        " name: string | null,",
        " roles: Array<MeRole>,",
        " permissions: Array<string>,",
        " branch: string,",
    ] {
        assert!(me.contains(kept), "Me has `{}`: {me}", kept.trim());
    }
    for gone in [" kind:", " role:", " username:"] {
        assert!(!me.contains(gone), "Me has no `{}`: {me}", gone.trim());
    }
    let role = text
        .lines()
        .find(|l| l.starts_with("export type MeRole = "))
        .unwrap_or_else(|| panic!("an `export type MeRole` line in {}", path.display()));
    assert!(role.contains(" role: string,"), "{role}");
    assert!(role.contains(" branchId: string | null,"), "{role}");
}

/// RBAC §11 C6h (#3323): an actor holding a custom group reads its installed permissions, not the base's seed.
#[actix_web::test]
async fn a_custom_group_reads_its_installed_permissions_not_the_base_seed() {
    static MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = MUTEX.lock().await;

    struct ResetSeeded;
    impl Drop for ResetSeeded {
        fn drop(&mut self) {
            RoleGrants::install(RoleGrantsSnapshot::seeded());
        }
    }
    let _reset = ResetSeeded;

    let custom_perms = vec![
        Permission::CompatRead,
        Permission::QueueOperate,
        Permission::SignedIn,
    ];
    let mut groups: Vec<(Group, Vec<Permission>)> = RoleGrants::BUILT_IN
        .into_iter()
        .map(|r| (Group::built_in(r), RoleGrants::seed(r)))
        .collect();
    groups.push((
        Group {
            code: "NIGHT_TELLER".to_string(),
            name: "Night Teller".to_string(),
            base: Role::Teller,
            built_in: false,
        },
        custom_perms.clone(),
    ));
    RoleGrants::install(RoleGrantsSnapshot::new(0, groups));

    let (desk, identity, app, reader) = setup().await;
    sqlx::query(
        "INSERT INTO roles (code, name, base_role, built_in) VALUES ('NIGHT_TELLER', 'Night Teller', 'TELLER', false)",
    )
    .execute(&desk.pool)
    .await
    .expect("insert custom role");

    sqlx::query(
        "INSERT INTO role_permission_grants (role_code, permission) VALUES ('NIGHT_TELLER', 'CompatRead'), ('NIGHT_TELLER', 'QueueOperate'), ('NIGHT_TELLER', 'SignedIn')",
    )
    .execute(&desk.pool)
    .await
    .expect("insert custom grants");

    let rows = [("NIGHT_TELLER", Some("B1"))];
    bound_user(&desk.pool, SUB_A, "ACTIVE", "Night Nok", &rows).await;
    let token = sign_in(&app, &identity, VALID_A).await;

    let data = me(&reader, &token).await;
    assert_eq!(keys(&data), me_keys(), "{data}");
    assert_eq!(data["branch"], json!("B1"));

    let permissions = strings(&data["permissions"]);
    let mut expected: Vec<String> = custom_perms.into_iter().map(name_of).collect();
    expected.sort();
    assert_eq!(permissions, expected);

    let teller_seed: Vec<String> = RoleGrants::seed(Role::Teller)
        .into_iter()
        .map(name_of)
        .collect();
    assert_ne!(
        permissions, teller_seed,
        "Me.permissions must reflect NIGHT_TELLER's set, not TELLER's seed"
    );

    drop(reader);
    desk.drop_schema().await;
}
