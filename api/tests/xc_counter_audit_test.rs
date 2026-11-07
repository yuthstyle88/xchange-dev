//! Ticket CA-1g (#6414): `branchCounterSave` writes an audit row for each saved counter change.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::{DateTime, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, MAIN, one_of_each_role, sign_in};
use serde_json::{Value, json};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const COUNTER_SAVE: &str = "/api/v3/commands/branch_counter_save";
const B2: &str = "B2";
const COUNTER_MAIN: &str = "c0000000-0000-4000-8000-000000000001";
const COUNTER_B2: &str = "c0000000-0000-4000-8000-0000000000b2";

fn t0() -> DateTime<Utc> {
    time("2026-11-06T03:00:00.000Z")
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    owner_id: String,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        desk.clock.set(t0());
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'สาขาสอง')")
            .bind(B2)
            .execute(&desk.db.pool)
            .await
            .expect("insert B2");
        let owner_id: String =
            sqlx::query_scalar("SELECT id::text FROM users WHERE name = 'owner'")
                .fetch_one(&desk.db.pool)
                .await
                .expect("owner user id");
        let room = Room {
            desk,
            app,
            cookies,
            owner_id,
        };
        room.counter(COUNTER_MAIN, MAIN, "เคาน์เตอร์ 1").await;
        room.counter(COUNTER_B2, B2, "สาขาสอง 1").await;
        room
    }

    async fn counter(&self, id: &str, branch: &str, name: &str) {
        let created = t0();
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, paired_at, last_seen_at, \
             counter_name) VALUES ($1::uuid, $2, 'Mac', $3, 'owner', $3, $3, $4)",
        )
        .bind(id)
        .bind(branch)
        .bind(created)
        .bind(name)
        .execute(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert counter {id}: {e}"));
    }

    async fn save_counter(&self, who: &str, body: Value) -> Answer {
        self.app
            .send(
                Call::post(ORIGIN, COUNTER_SAVE, body.to_string())
                    .header("idempotency-key", &uuid::Uuid::new_v4().to_string())
                    .cookie(&self.cookies[who]),
            )
            .await
    }

    async fn audit_rows(
        &self,
    ) -> Vec<(
        String,
        String,
        Option<String>,
        String,
        Option<Value>,
        Option<Value>,
    )> {
        sqlx::query_as(
            "SELECT actor_id, actor_role, branch_id, object_ref, before_redacted, after_redacted \
             FROM audit_events WHERE action = 'BRANCH_COUNTER_SAVE' ORDER BY id",
        )
        .fetch_all(&self.desk.db.pool)
        .await
        .expect("read audit_events")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn counter_body(agent: &str, name: &str, drawer: Value, scanner: Value, kiosk: Value) -> Value {
    json!({
        "branch": MAIN,
        "agentId": agent,
        "name": name,
        "cashDrawer": drawer,
        "scanner": scanner,
        "kioskLock": kiosk,
    })
}

#[actix_web::test]
async fn owner_setting_cash_drawer_true_writes_one_audit_row() {
    let room = Room::open().await;
    let answer = room
        .save_counter(
            "OWNER",
            counter_body(
                COUNTER_MAIN,
                "เคาน์เตอร์ 1",
                json!(true),
                Value::Null,
                Value::Null,
            ),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);

    let rows = room.audit_rows().await;
    assert_eq!(rows.len(), 1, "expected exactly one audit row: {rows:?}");
    let (actor_id, actor_role, branch_id, object_ref, before, after) = &rows[0];
    assert_eq!(actor_id, &room.owner_id);
    assert_eq!(actor_role, "EXECUTIVE");
    assert_eq!(branch_id.as_deref(), Some(MAIN));
    assert_eq!(object_ref, &format!("counter:{COUNTER_MAIN}"));
    assert_eq!(
        before,
        &Some(json!({
            "name": "เคาน์เตอร์ 1",
            "cashDrawer": null,
            "scanner": null,
            "kioskLock": null,
        }))
    );
    assert_eq!(
        after,
        &Some(json!({
            "name": "เคาน์เตอร์ 1",
            "cashDrawer": true,
            "scanner": null,
            "kioskLock": null,
        }))
    );
    room.finish().await;
}

#[actix_web::test]
async fn refused_saves_write_no_audit_row() {
    let room = Room::open().await;
    for who in ["MANAGER", "FRONT", "SYSTEM"] {
        let answer = room
            .save_counter(
                who,
                counter_body(
                    COUNTER_MAIN,
                    "เคาน์เตอร์ 1",
                    json!(true),
                    Value::Null,
                    Value::Null,
                ),
            )
            .await;
        assert_eq!(answer.status, 403, "{who}: {}", answer.text);
    }

    let other_branch = room
        .save_counter(
            "OWNER",
            counter_body(
                COUNTER_B2,
                "สาขาสอง 1",
                json!(true),
                Value::Null,
                Value::Null,
            ),
        )
        .await;
    assert_eq!(
        other_branch.status, 404,
        "other branch: {}",
        other_branch.text
    );

    assert!(
        room.audit_rows().await.is_empty(),
        "refused saves must write no audit row"
    );
    room.finish().await;
}
