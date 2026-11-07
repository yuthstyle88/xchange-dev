//! Ticket CA-1d (`docs/design/xc-counter-app.md` §4, §5.2, §5.4, §8.4): the Owner's per-counter settings. View
//! `GET /api/v3/views/branch_counters?branch=`, commands `POST /api/v3/commands/branch_counter_save` and
//! `POST /api/v3/commands/branch_cash_from_browser_save`. Written before the implementation. Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! | Call | Situation | Required result |
//! |---|---|---|
//! | view | a branch's counters | active counters of that branch only (revoked and other branches' left out), oldest first; every setting `null` when not set |
//! | view | no setting row | `cashFromBrowser: null`, `cashFromBrowserVersion: 0` |
//! | view | who may read | **the alert-settings view's rule**: each role gets the status `GET /api/v3/views/branch_alert_settings` gives it (policy row `branchAlertSettings`, `Permission::SettingsRead`) |
//! | `branchCounterSave` | Owner, each setting true/false/null and the name | read back by the view and in `device_agents`; the answer is `{counter}`, the saved list row |
//! | `branchCounterSave` | name with spaces around it | stored trimmed |
//! | `branchCounterSave` | name of 40 characters | accepted; 41 ⇒ refused and nothing written (characters, not bytes) |
//! | `branchCounterSave` | unknown, revoked or other-branch `agentId` | refused and nothing written |
//! | both commands | a non-owner (branch manager, teller, ADMIN without EXECUTIVE) | 403, nothing written (`Permission::BranchSettingsSave`) |
//! | `branchCashFromBrowserSave` | `expectedVersion: 0`, no row | creates version 1, `branch_settings` name `cashFromBrowser`, body `{"value": …}`; answers `{value, version}` |
//! | `branchCashFromBrowserSave` | stale `expectedVersion` | 409 `staleVersion` (what the v3 commands answer for the store's version conflict), nothing written |
//! | `branchCashFromBrowserSave` | `value: null` | allowed: back to not set, and the version still moves |
//!
//! Through the server's whole route table with real sessions (`rpc_common`): `OWNER` holds EXECUTIVE and ADMIN,
//! `FRONT` is a teller of MAIN, `MANAGER` a branch manager of MAIN, `SYSTEM` an ADMIN of MAIN without EXECUTIVE.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, MAIN, one_of_each_role, sign_in};
use serde_json::{Value, json};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const VIEW: &str = "/api/v3/views/branch_counters";
const ALERT_VIEW: &str = "/api/v3/views/branch_alert_settings";
const COUNTER_SAVE: &str = "/api/v3/commands/branch_counter_save";
const CASH_SAVE: &str = "/api/v3/commands/branch_cash_from_browser_save";
const B2: &str = "B2";

const OLDEST: &str = "c0000000-0000-4000-8000-00000000000f";
const NEWER: &str = "c0000000-0000-4000-8000-000000000001";
const REVOKED: &str = "c0000000-0000-4000-8000-0000000000dd";
const OF_B2: &str = "c0000000-0000-4000-8000-0000000000b2";
const UNKNOWN: &str = "c0000000-0000-4000-8000-0000000000ee";

fn t0() -> DateTime<Utc> {
    time("2026-11-06T03:00:00.000Z")
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// MAIN with two active counters (the older one has the larger id) and a revoked one; B2 with one counter.
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
        let room = Room { desk, app, cookies };
        room.counter(OLDEST, MAIN, 0, "เคาน์เตอร์ 1").await;
        room.counter(NEWER, MAIN, 60, "เคาน์เตอร์ 2").await;
        room.counter(REVOKED, MAIN, 30, "เก่า").await;
        room.counter(OF_B2, B2, 10, "สาขาสอง 1").await;
        sqlx::query(
            "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = $1::uuid",
        )
        .bind(REVOKED)
        .execute(&room.desk.db.pool)
        .await
        .expect("revoke a counter");
        room
    }

    async fn counter(&self, id: &str, branch: &str, age: i64, name: &str) {
        let created = t0() + Duration::seconds(age);
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

    async fn get(&self, who: &str, path: &str) -> Answer {
        let mut call = Call::post(ORIGIN, path, Vec::new()).cookie(&self.cookies[who]);
        call.method = Method::GET;
        self.app.send(call).await
    }

    async fn view(&self, who: &str, branch: &str) -> Answer {
        self.get(who, &format!("{VIEW}?branch={branch}")).await
    }

    /// The `data` of a 200 from the view, as the Owner.
    async fn data(&self, branch: &str) -> Value {
        let answer = self.view("OWNER", branch).await;
        assert_eq!(answer.status, 200, "the view: {}", answer.text);
        answer.json()["data"].clone()
    }

    async fn command(&self, who: &str, path: &str, body: Value) -> Answer {
        self.app
            .send(
                Call::post(ORIGIN, path, body.to_string())
                    .header("idempotency-key", &uuid::Uuid::new_v4().to_string())
                    .cookie(&self.cookies[who]),
            )
            .await
    }

    async fn save_counter(&self, who: &str, body: Value) -> Answer {
        self.command(who, COUNTER_SAVE, body).await
    }

    async fn save_cash(&self, who: &str, body: Value) -> Answer {
        self.command(who, CASH_SAVE, body).await
    }

    /// (`counter_name`, `cash_drawer`, `scanner`, `kiosk_lock`) as stored.
    async fn stored(&self, id: &str) -> (String, Option<bool>, Option<bool>, Option<bool>) {
        sqlx::query_as(
            "SELECT counter_name, cash_drawer, scanner, kiosk_lock FROM device_agents WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.desk.db.pool)
        .await
        .expect("the counter's row")
    }

    /// (`version`, `body`) of the branch's `cashFromBrowser` row.
    async fn cash_row(&self, branch: &str) -> Option<(i64, Value)> {
        sqlx::query_as(
            "SELECT version, body FROM branch_settings WHERE branch_id = $1 AND name = 'cashFromBrowser'",
        )
        .bind(branch)
        .fetch_optional(&self.desk.db.pool)
        .await
        .expect("read branch_settings")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn counter_body(agent: &str, name: &str, drawer: Value, scanner: Value, kiosk: Value) -> Value {
    json!({ "branch": MAIN, "agentId": agent, "name": name, "cashDrawer": drawer, "scanner": scanner,
            "kioskLock": kiosk })
}

/// Fails unless this is a refusal the seam answered: a 4xx with `error.code`. The routes do not exist on the skeleton,
/// and a 404 without a code is not a refusal.
fn assert_refused(answer: &Answer, label: &str) {
    assert!(
        (400..500).contains(&answer.status),
        "{label}: {} {}",
        answer.status,
        answer.text
    );
    assert!(
        answer.json()["error"]["code"].is_string(),
        "{label}: a v3 refusal with error.code: {}",
        answer.text
    );
}

fn agent_ids(data: &Value) -> Vec<String> {
    data["counters"]
        .as_array()
        .expect("counters")
        .iter()
        .map(|c| c["agentId"].as_str().expect("agentId").to_owned())
        .collect()
}

// ── the view ────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_view_lists_the_branchs_active_counters_oldest_first() {
    let room = Room::open().await;
    let data = room.data(MAIN).await;
    assert_eq!(
        agent_ids(&data),
        [OLDEST, NEWER],
        "oldest first; the revoked one and B2's are left out"
    );
    assert_eq!(agent_ids(&room.data(B2).await), [OF_B2]);
    room.finish().await;
}

/// §8.4's row: every setting `boolean | null`, null = not set; the view does not resolve them (that is `info`'s job).
#[actix_web::test]
async fn the_view_answers_null_for_every_setting_not_set() {
    let room = Room::open().await;
    let data = room.data(MAIN).await;
    let first = &data["counters"][0];
    assert_eq!(first["agentId"], OLDEST);
    assert_eq!(first["name"], "เคาน์เตอร์ 1");
    assert_eq!(first["cashDrawer"], Value::Null);
    assert_eq!(first["scanner"], Value::Null);
    assert_eq!(first["kioskLock"], Value::Null);
    assert!(first["pairedAt"].is_string(), "pairedAt: {first}");
    assert!(first["lastSeenAt"].is_string(), "lastSeenAt: {first}");
    room.finish().await;
}

#[actix_web::test]
async fn the_view_answers_not_set_and_version_0_with_no_setting_row() {
    let room = Room::open().await;
    let data = room.data(MAIN).await;
    assert_eq!(data["cashFromBrowser"], Value::Null);
    assert_eq!(data["cashFromBrowserVersion"], 0);
    assert_eq!(room.cash_row(MAIN).await, None);
    room.finish().await;
}

/// The view follows the alert-settings view's rule: every role gets the status that view gives it.
#[actix_web::test]
async fn the_view_is_readable_by_whom_the_alert_settings_view_allows() {
    let room = Room::open().await;
    for who in [
        "OWNER", "FRONT", "MANAGER", "SYSTEM", "FINANCE", "RATES", "MONITOR",
    ] {
        let alert = room.get(who, &format!("{ALERT_VIEW}?branch={MAIN}")).await;
        let answer = room.view(who, MAIN).await;
        assert_eq!(
            answer.status, alert.status,
            "{who}: the alert view answers {}, this one {}",
            alert.text, answer.text
        );
    }
    room.finish().await;
}

// ── saving a counter ────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_owner_saves_every_setting_and_the_name() {
    let room = Room::open().await;
    let answer = room
        .save_counter(
            "OWNER",
            counter_body(OLDEST, "หน้าร้าน", json!(true), json!(false), Value::Null),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);

    assert_eq!(
        room.stored(OLDEST).await,
        ("หน้าร้าน".to_owned(), Some(true), Some(false), None)
    );

    let data = room.data(MAIN).await;
    let row = &data["counters"][0];
    assert_eq!(row["agentId"], OLDEST);
    assert_eq!(row["name"], "หน้าร้าน");
    assert_eq!(row["cashDrawer"], true);
    assert_eq!(row["scanner"], false);
    assert_eq!(row["kioskLock"], Value::Null);
    assert_eq!(
        &answer.json()["counter"],
        row,
        "§8.4: the answer is one list row"
    );

    assert_eq!(
        room.stored(NEWER).await,
        ("เคาน์เตอร์ 2".to_owned(), None, None, None),
        "the other counter is left alone"
    );
    room.finish().await;
}

/// Each setting takes each of its three values; null is how the Owner goes back to "ค่าเริ่มต้น" (§5.2).
#[actix_web::test]
async fn each_setting_takes_true_false_and_null() {
    let room = Room::open().await;
    for (drawer, scanner, kiosk, stored) in [
        (
            json!(false),
            json!(true),
            json!(false),
            (Some(false), Some(true), Some(false)),
        ),
        (
            json!(true),
            json!(false),
            json!(true),
            (Some(true), Some(false), Some(true)),
        ),
        (Value::Null, Value::Null, Value::Null, (None, None, None)),
    ] {
        let answer = room
            .save_counter(
                "OWNER",
                counter_body(OLDEST, "A", drawer.clone(), scanner.clone(), kiosk.clone()),
            )
            .await;
        assert_eq!(
            answer.status, 200,
            "{drawer} {scanner} {kiosk}: {}",
            answer.text
        );
        assert_eq!(
            room.stored(OLDEST).await,
            ("A".to_owned(), stored.0, stored.1, stored.2),
            "{drawer} {scanner} {kiosk}"
        );
        let row = room.data(MAIN).await["counters"][0].clone();
        assert_eq!(row["cashDrawer"], drawer);
        assert_eq!(row["scanner"], scanner);
        assert_eq!(row["kioskLock"], kiosk);
    }
    room.finish().await;
}

#[actix_web::test]
async fn the_name_is_stored_trimmed() {
    let room = Room::open().await;
    let answer = room
        .save_counter(
            "OWNER",
            counter_body(OLDEST, "  หน้าร้าน \t", Value::Null, Value::Null, Value::Null),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(room.stored(OLDEST).await.0, "หน้าร้าน");
    assert_eq!(answer.json()["counter"]["name"], "หน้าร้าน");
    room.finish().await;
}

/// §8.4 / `CounterSettings.name`: at most 40 characters, after trimming, counted as characters.
#[actix_web::test]
async fn a_name_of_40_characters_is_taken_and_41_is_refused() {
    let room = Room::open().await;
    let forty = "ก".repeat(40);
    let answer = room
        .save_counter(
            "OWNER",
            counter_body(
                OLDEST,
                &format!(" {forty} "),
                json!(true),
                Value::Null,
                Value::Null,
            ),
        )
        .await;
    assert_eq!(answer.status, 200, "40 characters: {}", answer.text);
    assert_eq!(room.stored(OLDEST).await.0, forty);

    let answer = room
        .save_counter(
            "OWNER",
            counter_body(
                OLDEST,
                &"ข".repeat(41),
                json!(false),
                Value::Null,
                Value::Null,
            ),
        )
        .await;
    assert_refused(&answer, "41 characters");
    assert_eq!(
        room.stored(OLDEST).await,
        (forty, Some(true), None, None),
        "nothing written"
    );
    room.finish().await;
}

#[actix_web::test]
async fn an_unknown_revoked_or_other_branch_counter_is_refused_and_nothing_is_written() {
    let room = Room::open().await;
    // The command works, so what follows is its refusal and not a missing route.
    let answer = room
        .save_counter(
            "OWNER",
            counter_body(OLDEST, "ก", json!(true), Value::Null, Value::Null),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);

    for (label, agent) in [("unknown", UNKNOWN), ("revoked", REVOKED), ("of B2", OF_B2)] {
        let answer = room
            .save_counter(
                "OWNER",
                counter_body(agent, "ใหม่", json!(true), json!(true), json!(true)),
            )
            .await;
        assert_refused(&answer, label);
    }
    assert_eq!(
        room.stored(REVOKED).await,
        ("เก่า".to_owned(), None, None, None)
    );
    assert_eq!(
        room.stored(OF_B2).await,
        ("สาขาสอง 1".to_owned(), None, None, None)
    );
    let unknown: i64 = sqlx::query_scalar("SELECT count(*) FROM device_agents WHERE id = $1::uuid")
        .bind(UNKNOWN)
        .fetch_one(&room.desk.db.pool)
        .await
        .expect("count");
    assert_eq!(unknown, 0);
    room.finish().await;
}

// ── who may save ────────────────────────────────────────────────────────────

/// §4: only the Owner saves these. ADMIN without EXECUTIVE is no owner (`Permission::BranchSettingsSave`).
#[actix_web::test]
async fn a_non_owner_gets_403_from_both_commands_and_nothing_is_written() {
    let room = Room::open().await;
    for who in ["MANAGER", "FRONT", "SYSTEM"] {
        let answer = room
            .save_counter(
                who,
                counter_body(OLDEST, "ใหม่", json!(true), json!(true), json!(true)),
            )
            .await;
        assert_eq!(
            answer.status, 403,
            "{who} branchCounterSave: {}",
            answer.text
        );
        let answer = room
            .save_cash(
                who,
                json!({ "branch": MAIN, "expectedVersion": 0, "value": false }),
            )
            .await;
        assert_eq!(
            answer.status, 403,
            "{who} branchCashFromBrowserSave: {}",
            answer.text
        );
    }
    assert_eq!(
        room.stored(OLDEST).await,
        ("เคาน์เตอร์ 1".to_owned(), None, None, None)
    );
    assert_eq!(room.cash_row(MAIN).await, None);
    room.finish().await;
}

// ── cashFromBrowser ─────────────────────────────────────────────────────────

#[actix_web::test]
async fn expected_version_0_creates_version_1() {
    let room = Room::open().await;
    let answer = room
        .save_cash(
            "OWNER",
            json!({ "branch": MAIN, "expectedVersion": 0, "value": false }),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.json(), json!({ "value": false, "version": 1 }));
    assert_eq!(
        room.cash_row(MAIN).await,
        Some((1, json!({ "value": false })))
    );

    let data = room.data(MAIN).await;
    assert_eq!(data["cashFromBrowser"], false);
    assert_eq!(data["cashFromBrowserVersion"], 1);
    assert_eq!(room.cash_row(B2).await, None, "per branch");
    room.finish().await;
}

#[actix_web::test]
async fn a_stale_expected_version_is_refused_and_nothing_is_written() {
    let room = Room::open().await;
    let answer = room
        .save_cash(
            "OWNER",
            json!({ "branch": MAIN, "expectedVersion": 0, "value": true }),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);

    for stale in [0, 2] {
        let answer = room
            .save_cash(
                "OWNER",
                json!({ "branch": MAIN, "expectedVersion": stale, "value": false }),
            )
            .await;
        assert_eq!(answer.status, 409, "expected {stale}: {}", answer.text);
        assert_eq!(
            answer.json()["error"]["code"],
            "staleVersion",
            "expected {stale}: {}",
            answer.text
        );
    }
    assert_eq!(
        room.cash_row(MAIN).await,
        Some((1, json!({ "value": true })))
    );
    room.finish().await;
}

/// §5.4: "not set" is a value the Owner can go back to; the view shows null again, and the version moves on.
#[actix_web::test]
async fn value_null_goes_back_to_not_set() {
    let room = Room::open().await;
    let first = room
        .save_cash(
            "OWNER",
            json!({ "branch": MAIN, "expectedVersion": 0, "value": false }),
        )
        .await;
    assert_eq!(first.status, 200, "{}", first.text);

    let answer = room
        .save_cash(
            "OWNER",
            json!({ "branch": MAIN, "expectedVersion": 1, "value": null }),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.json(), json!({ "value": null, "version": 2 }));
    assert_eq!(
        room.cash_row(MAIN).await,
        Some((2, json!({ "value": null })))
    );

    let data = room.data(MAIN).await;
    assert_eq!(data["cashFromBrowser"], Value::Null);
    assert_eq!(data["cashFromBrowserVersion"], 2);
    room.finish().await;
}
