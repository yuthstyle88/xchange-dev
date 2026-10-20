// Written before the fix by claude/d7aa64. Do not edit, delete or weaken.

//! `bookingPolicySet` writes the organisation-wide booking policy and needs organisation-wide scope.
//!
//! `PUT /api/booking-policy` (`commands/bookings/policy.rs`) upserts the one `booking_policies` row keyed on
//! `who.organization_id`. Its policy row needs `BranchesManage` (`xc-auth` `areas/bookings.rs`), which only
//! `Role::Admin` holds (`RoleGrants::seed` / §11.2). The seam authorises against the body `branch`, so an ADMIN
//! whose only role row is on XC-B01 is granted XC-B01 and today rewrites the whole organisation's policy.
//!
//! Required (lane-controller ruling, security): an organisation-wide policy write needs an
//! organisation-wide grant (a `BranchesManage` role row with no branch). A branch-scoped ADMIN gets
//! `403 outOfScope` and the policy row is unchanged; an organisation-wide ADMIN still gets 200.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use bk6_common::*;

/// An ADMIN whose only role row is on XC-B01 (`ScopedRole(Admin, Some(XC-B01))`).
const B01_ADMIN: &str = "ADMIN@XC-B01";
/// An organisation-wide ADMIN (`ScopedRole(Admin, None)`).
const ORG_ADMIN: &str = "ADMIN@*";

type PolicyRow = (i32, i32, i32);

async fn policy_of(pool: &PgPool) -> Option<PolicyRow> {
    sqlx::query_as(
        "SELECT cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes \
         FROM booking_policies WHERE organization_id = $1::uuid",
    )
    .bind(ORG)
    .fetch_optional(pool)
    .await
    .expect("read booking policy")
}

fn new_policy(branch: Option<&str>) -> Value {
    let mut body = json!({
        "cancelCutoffMinutes": 45,
        "noShowGraceMinutes": 20,
        "confirmExpiryMinutes": 30,
    });
    if let Some(b) = branch {
        body["branch"] = json!(b);
    }
    body
}

async fn set_policy(db: &TestDb, who: &str, body: Value) -> Answer {
    call(
        db,
        Some(who),
        Method::PUT,
        "/api/booking-policy",
        Some(body),
        Some(&format!("k-pol-{}", Uuid::new_v4())),
    )
    .await
}

fn out_of_scope(a: &Answer, what: &str) {
    assert!(
        a.status == StatusCode::FORBIDDEN && a.body["error"]["code"] == "outOfScope",
        "{what}: expected 403 outOfScope; got {} {}",
        a.status,
        a.text
    );
}

// ── row 1: a branch-scoped ADMIN cannot change the organisation's policy ────

#[actix_web::test]
async fn a_branch_scoped_admin_naming_its_branch_cannot_set_the_org_policy() {
    let db = TestDb::start().await;
    let before = policy_of(&db.pool).await;
    assert_ne!(before, Some((45, 20, 30)), "seeded policy must differ");

    let a = set_policy(&db, B01_ADMIN, new_policy(Some(B01))).await;

    out_of_scope(&a, "bookingPolicySet by an XC-B01 ADMIN naming XC-B01");
    assert_eq!(
        policy_of(&db.pool).await,
        before,
        "the organisation's booking policy must be unchanged"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_branch_scoped_admin_naming_no_branch_cannot_set_the_org_policy() {
    let db = TestDb::start().await;
    let before = policy_of(&db.pool).await;

    let a = set_policy(&db, B01_ADMIN, new_policy(None)).await;

    out_of_scope(&a, "bookingPolicySet by an XC-B01 ADMIN naming no branch");
    assert_eq!(
        policy_of(&db.pool).await,
        before,
        "the organisation's booking policy must be unchanged"
    );
    db.finish().await;
}

// ── row 2 (keep-pin): an organisation-wide ADMIN still sets it ──────────────

#[actix_web::test]
async fn an_org_wide_admin_still_sets_the_org_policy() {
    let db = TestDb::start().await;

    let a = set_policy(&db, ORG_ADMIN, new_policy(Some(B01))).await;

    assert_eq!(
        a.status,
        StatusCode::OK,
        "bookingPolicySet by an organisation-wide ADMIN: got {} {}",
        a.status,
        a.text
    );
    assert_eq!(policy_of(&db.pool).await, Some((45, 20, 30)));
    db.finish().await;
}
