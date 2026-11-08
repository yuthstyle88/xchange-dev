//! Up-front test for OL-7 device-events-names (verifier row 98: "/devices audit table still shows truncated UUIDs";
//! sub-controller claude/1cc51615's decision, /tmp/ol7/ORDER-claude-19-b.md item 3). Written by claude/94389a6b before
//! the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `GET /api/v3/views/device_events` answers each line's `actor` as stored
//! (`api/src/presentation/http/v3/views/device_events.rs`), and a staff action stores the user's id
//! (`api/src/presentation/http/devices.rs`, `actor_id = callers.user…user_id`), so the devices screen can only print a
//! UUID. Each row gains `actorName`: the user's `users.name` when `actor` is a user id
//! (`xc_auth::application::user::directory::user_names`), else `null`. `actor` itself is unchanged.
//!
//! Real sessions through the whole route table (`rpc_common`), as `devices_view_org_scope_test.rs` does. Needs
//! `TEST_DATABASE_URL`; nothing skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use actix_web::http::Method;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const AGENT: &str = "agent:7b1c2a90-0000-4000-8000-000000000001";
const KIOSK_NAME: &str = "ลูกค้า (ตู้)";
const UNKNOWN_UUID: &str = "00000000-0000-0000-0000-00000000dead";

fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    anna: String,
}

impl Room {
    /// The owner and one employee of every staff role in `MAIN`; a user "Anna Teller" in MAIN's organisation; and four
    /// lines in MAIN's device log: by Anna's id, by an agent, by a kiosk (a name, as kiosk lines store), and by a UUID
    /// that is no user.
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;

        let anna: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, 'Anna Teller', 'ACTIVE', 'test' FROM branches WHERE id = 'MAIN' \
             RETURNING id::text",
        )
        .fetch_one(&desk.db.pool)
        .await
        .expect("insert Anna");

        for (i, actor) in [anna.as_str(), AGENT, KIOSK_NAME, UNKNOWN_UUID]
            .iter()
            .enumerate()
        {
            sqlx::query(
                "INSERT INTO device_events (branch_id, actor, kind, ref, detail, at) \
                 VALUES ('MAIN', $1, 'PRINT_RECEIPT', $2, '{}'::jsonb, now() + make_interval(secs => $3))",
            )
            .bind(actor)
            .bind(format!("REF-{i}"))
            .bind(i as f64)
            .execute(&desk.db.pool)
            .await
            .expect("insert a device line");
        }
        Room {
            desk,
            app,
            cookies,
            anna,
        }
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        self.app.send(get(path).cookie(&self.cookies[who])).await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// The page's row whose `ref` is `reference`.
fn row<'a>(rows: &'a [Value], reference: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["ref"] == json!(reference))
        .unwrap_or_else(|| panic!("a row with ref {reference} in {rows:?}"))
}

#[actix_web::test]
async fn each_device_line_names_its_actor_when_the_actor_is_a_user() {
    let room = Room::open().await;
    let answer = room
        .get("OWNER", "/api/v3/views/device_events?branch=MAIN")
        .await;
    assert_eq!(answer.status, 200, "{:.400}", answer.text);
    let body = answer.json();
    let rows = body["data"].as_array().expect("a page of rows").clone();

    let by_user = row(&rows, "REF-0");
    assert_eq!(
        by_user["actor"],
        json!(room.anna),
        "actor stays the stored id"
    );
    assert_eq!(
        by_user["actorName"],
        json!("Anna Teller"),
        "a user id is named: {by_user}"
    );

    for reference in ["REF-1", "REF-2", "REF-3"] {
        let r = row(&rows, reference);
        assert!(
            r.as_object().is_some_and(|o| o.contains_key("actorName")),
            "{reference}: every row carries actorName: {r}"
        );
        assert_eq!(
            r["actorName"],
            Value::Null,
            "{reference}: not a user id, so no name: {r}"
        );
    }
    assert_eq!(
        row(&rows, "REF-1")["actor"],
        json!(AGENT),
        "actor stays as stored"
    );
    assert_eq!(
        row(&rows, "REF-2")["actor"],
        json!(KIOSK_NAME),
        "actor stays as stored"
    );
    assert_eq!(
        row(&rows, "REF-3")["actor"],
        json!(UNKNOWN_UUID),
        "actor stays as stored"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_disabled_users_lines_still_name_them() {
    let room = Room::open().await;
    sqlx::query("UPDATE users SET status = 'DISABLED' WHERE id = $1::uuid")
        .bind(&room.anna)
        .execute(&room.desk.db.pool)
        .await
        .expect("disable Anna");
    let answer = room
        .get("OWNER", "/api/v3/views/device_events?branch=MAIN")
        .await;
    assert_eq!(answer.status, 200, "{:.400}", answer.text);
    let body = answer.json();
    let rows = body["data"].as_array().expect("a page of rows").clone();
    assert_eq!(row(&rows, "REF-0")["actorName"], json!("Anna Teller"));
    room.finish().await;
}
