//! OR-7.m S-D3 (#3716): `CompatRpc::colleagues` and `ServiceSnapshotSources::people`
//! read `users` + `user_roles` at the branch (valid at `now`, non-`DISABLED`,
//! highest role per `STAFF_PRIORITY`, org-wide users excluded).

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use compat_common::Person;
use rpc_common::{Call, Desk, MAIN, STAFF_RPC, sign_in};
use serde_json::{Value, json};
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::ids::BranchId;
use xchange::application::compat::snapshot_sources::SnapshotSources;
use xchange::application::compat::v2_json::PersonLabel;
use xchange::infrastructure::compat::snapshot_sources::ServiceSnapshotSources;

const ORIGIN: &str = "http://desk.test";

fn sources(desk: &Desk) -> ServiceSnapshotSources {
    let transactions = TransactionService::new(
        Arc::new(PgTransactionRepository::new(desk.db.pool.clone())),
        desk.db.branch_tx(),
    );
    ServiceSnapshotSources::new(transactions)
}

async fn org_id(desk: &Desk) -> String {
    sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
        .fetch_one(&desk.db.pool)
        .await
        .expect("the XC organization")
}

/// A user whose role at `MAIN` ended (`valid_to` in the past) and the org-wide
/// owner are not listed in either the staff view's `employees` or the snapshot
/// source's `people`.
#[actix_web::test]
async fn an_ended_role_at_the_branch_is_not_listed() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let active_id = "00000000-0000-4000-8000-000000000301";
    let ended_id = "00000000-0000-4000-8000-000000000302";
    let people = vec![
        Person {
            key: "FRONT".to_string(),
            kind: "staff".to_string(),
            id: active_id.to_string(),
            username: "active_teller".to_string(),
            name: "Active Teller".to_string(),
            role: "FRONT".to_string(),
            branch_id: Some(MAIN.to_string()),
            must_change: false,
        },
        Person {
            key: "ENDED".to_string(),
            kind: "staff".to_string(),
            id: ended_id.to_string(),
            username: "ended_teller".to_string(),
            name: "Ended Teller".to_string(),
            role: "FRONT".to_string(),
            branch_id: Some(MAIN.to_string()),
            must_change: false,
        },
    ];
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people[..1]).await;

    sqlx::query("UPDATE user_roles SET valid_to = $1 WHERE user_id = $2::uuid")
        .bind(Utc::now() - Duration::hours(1))
        .bind(ended_id)
        .execute(&desk.db.pool)
        .await
        .expect("end the second user's role");

    let answer = app
        .send(Call::action(ORIGIN, STAFF_RPC, "adminSnapshot", json!({})).cookie(&cookies["FRONT"]))
        .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    let listed: Vec<Value> = answer.json()["employees"]
        .as_array()
        .expect("employees")
        .clone();
    assert_eq!(
        listed,
        vec![json!({
            "id": active_id,
            "name": "active_teller",
            "role": "FRONT",
        })]
    );

    let snapshot_people = sources(&desk)
        .people(&BranchId::new(MAIN))
        .await
        .expect("people of MAIN");
    assert_eq!(snapshot_people.len(), 1);
    assert_eq!(
        snapshot_people.get(active_id),
        Some(&PersonLabel {
            name: "active_teller".to_string(),
            role: "FRONT".to_string(),
        })
    );
    assert!(!snapshot_people.contains_key(ended_id));

    desk.finish().await;
}

/// A user holding both `TELLER` and `BRANCH_MANAGER` at `MAIN` is listed once,
/// with the higher-priority role from `STAFF_PRIORITY` (`TELLER` → `FRONT`).
#[actix_web::test]
async fn a_user_with_two_roles_at_the_branch_is_listed_once_by_staff_priority() {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let dual_id = "00000000-0000-4000-8000-000000000303";
    let people = vec![Person {
        key: "DUAL".to_string(),
        kind: "staff".to_string(),
        id: dual_id.to_string(),
        username: "dual_staff".to_string(),
        name: "Dual Staff".to_string(),
        role: "MANAGER".to_string(),
        branch_id: Some(MAIN.to_string()),
        must_change: false,
    }];
    desk.employ(&people).await;

    let org = org_id(&desk).await;
    let mut conn = desk.db.pool.acquire().await.expect("acquire connection");
    xc_testdb::people::seed_roles(
        &mut conn,
        dual_id,
        &org,
        &[xc_testdb::people::RoleRow {
            role_code: "TELLER",
            branch_id: Some(MAIN),
            valid_from: DateTime::<Utc>::UNIX_EPOCH,
            valid_to: None,
        }],
    )
    .await
    .expect("seed second role at MAIN");
    drop(conn);

    let cookies = sign_in(&app, ORIGIN, &people).await;
    let answer = app
        .send(Call::action(ORIGIN, STAFF_RPC, "adminSnapshot", json!({})).cookie(&cookies["DUAL"]))
        .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    let listed: Vec<Value> = answer.json()["employees"]
        .as_array()
        .expect("employees")
        .clone();
    assert_eq!(
        listed,
        vec![json!({
            "id": dual_id,
            "name": "dual_staff",
            "role": "FRONT",
        })]
    );

    let snapshot_people = sources(&desk)
        .people(&BranchId::new(MAIN))
        .await
        .expect("people of MAIN");
    assert_eq!(snapshot_people.len(), 1);
    assert_eq!(
        snapshot_people.get(dual_id),
        Some(&PersonLabel {
            name: "dual_staff".to_string(),
            role: "FRONT".to_string(),
        })
    );

    desk.finish().await;
}
