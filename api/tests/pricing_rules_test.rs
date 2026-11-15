//! xchange#6555 (GA-1, `docs/design/blueprint/owner-set-values.md` §2, §6): the Owner's pricing rules, saved as
//! versions through `pricingRulesSave` and read through the `pricing_rules` view.
//!
//! | # | Case | Result |
//! |---|---|---|
//! | not set | no `pricing_rule_versions` row; read | `current` has every field `null`, `history` is empty |
//! | G-17 | FINANCE, BRANCH_MANAGER, ADMIN (no EXECUTIVE) save | refused `roleNotAllowedHere` (the policy row's), no row |
//! | G-18 | EXECUTIVE saves twice | two versions; the view answers the newer with `setBy`/`createdAt`, history lists both, newest first |
//! | merge | a key left out / sent as `null` | kept from the version before / cleared |
//! | validation | a range with min > max, a 0 %, bands not from 0 | 400 `validationFailed` naming the field, no row |
//! | concurrent | two saves from two branches, held back together at the insert, with different keys | both land, in order; the newer holds both keys |
//! | read | a teller (holds `SettingsRead`) | reads the view |
//!
//! Through the server's whole route table with real sessions (`rpc_common`): `OWNER` holds EXECUTIVE. Needs
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
const SAVE: &str = "/api/pricing/rules";
const VIEW: &str = "/api/v3/views/pricing_rules?branch=MAIN";

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        // "not set" starts empty: remove the fixture rule `snapshot_common` saves (#6928), as G-9 does.
        sqlx::query("DELETE FROM pricing_rule_versions WHERE organization_id = '00000000-0000-0000-0000-000000000001'")
            .execute(&desk.db.pool)
            .await
            .expect("delete the fixture rule");
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room { desk, app, cookies }
    }

    fn save_call(&self, who: &str, body: Value) -> Call {
        Call::post(ORIGIN, SAVE, body.to_string())
            .header("idempotency-key", &uuid::Uuid::new_v4().to_string())
            .cookie(&self.cookies[who])
    }

    async fn save(&self, who: &str, body: Value) -> Answer {
        self.app.send(self.save_call(who, body)).await
    }

    async fn read(&self, who: &str) -> Answer {
        let mut call = Call::post(ORIGIN, VIEW, Vec::<u8>::new())
            .without("x-xchange-request")
            .without("content-type")
            .without("origin")
            .cookie(&self.cookies[who]);
        call.method = Method::GET;
        self.app.send(call).await
    }

    /// The view's `data`, after asserting a 200.
    async fn view(&self, who: &str) -> Value {
        let answer = self.read(who).await;
        assert_eq!(answer.status, 200, "read as {who}: {}", answer.text);
        answer.json()["data"].clone()
    }

    async fn rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM pricing_rule_versions")
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("count the versions")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// A command's 200 output, wherever the envelope puts it.
fn output_of(answer: &Answer) -> Value {
    let body = answer.json();
    if body.get("data").is_some() {
        body["data"].clone()
    } else {
        body
    }
}

fn field_of(answer: &Answer) -> String {
    answer.json()["error"]["field"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

const RULE_KEYS: [&str; 9] = [
    "farFromAnchorLimitPct",
    "largeMovePct",
    "rateDecimals",
    "thbCashStep",
    "branchBpCap",
    "betterPctRange",
    "minSpreadPctRange",
    "marketStaleAfterMinutes",
    "advice",
];

#[actix_web::test]
async fn an_organisation_with_no_save_reads_not_set() {
    let room = Room::open().await;
    let data = room.view("OWNER").await;
    let current = &data["current"];
    for key in ["version", "setBy", "setByName", "createdAt"] {
        assert!(current[key].is_null(), "{key}: {data}");
    }
    for key in RULE_KEYS {
        assert!(current["rules"][key].is_null(), "{key} is not set: {data}");
    }
    assert_eq!(data["history"], json!([]));
    room.finish().await;
}

#[actix_web::test]
async fn g17_only_executive_saves() {
    let room = Room::open().await;
    for who in ["FINANCE", "MANAGER", "SYSTEM"] {
        let answer = room.save(who, json!({ "largeMovePct": "1" })).await;
        assert_eq!(
            answer.json()["error"]["code"],
            "roleNotAllowedHere",
            "{who}: {}",
            answer.text
        );
    }
    assert_eq!(room.rows().await, 0, "a refused save writes nothing");
    room.finish().await;
}

#[actix_web::test]
async fn g18_two_saves_are_two_versions_and_the_newer_is_current() {
    let room = Room::open().await;
    let first = json!({
        "farFromAnchorLimitPct": "3",
        "largeMovePct": "1",
        "rateDecimals": [
            { "fromPrice": "0", "decimals": 4 },
            { "fromPrice": "1", "decimals": 3 },
            { "fromPrice": "10", "decimals": 2 }
        ],
        "thbCashStep": "0.25",
        "branchBpCap": 300,
        "betterPctRange": { "min": "-2", "max": "2" },
        "minSpreadPctRange": { "min": "0", "max": "5" },
        "marketStaleAfterMinutes": 30,
        "advice": { "farPct": "1", "spreadRatio": "1.5", "spreadGapPt": "0.5", "marketMovedPct": "0.2" }
    });
    let answer = room.save("OWNER", first.clone()).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let saved = output_of(&answer);
    assert!(saved["setBy"].is_string(), "{saved}");
    assert_eq!(
        saved["setByName"], "owner",
        "the seed names the OWNER user `owner`"
    );
    assert_eq!(saved["rules"], first, "the saved rules read back as sent");

    // Left out: kept. `null`: cleared. A value: replaced.
    let answer = room
        .save("OWNER", json!({ "largeMovePct": "2", "thbCashStep": null }))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let second = output_of(&answer);

    assert_eq!(room.rows().await, 2);
    let data = room.view("OWNER").await;
    let current = &data["current"];
    assert_eq!(current["version"], second["version"]);
    assert_ne!(current["version"], saved["version"]);
    assert_eq!(current["setBy"], saved["setBy"]);
    assert_eq!(current["setByName"], "owner");
    assert!(current["createdAt"].is_string(), "{data}");
    assert_eq!(current["rules"]["largeMovePct"], "2");
    assert!(current["rules"]["thbCashStep"].is_null(), "cleared: {data}");
    assert_eq!(
        current["rules"]["farFromAnchorLimitPct"], "3",
        "kept: {data}"
    );
    assert_eq!(current["rules"]["advice"], first["advice"], "kept: {data}");

    let history = data["history"].as_array().expect("a history");
    let versions: Vec<&Value> = history.iter().map(|v| &v["version"]).collect();
    assert_eq!(
        versions,
        [&second["version"], &saved["version"]],
        "newest first"
    );
    assert_eq!(
        history[1]["rules"]["thbCashStep"], "0.25",
        "an old version never changes"
    );
    room.finish().await;
}

#[actix_web::test]
async fn an_invalid_rule_is_refused_naming_its_field() {
    let room = Room::open().await;
    for (body, field) in [
        (
            json!({ "betterPctRange": { "min": "1", "max": "-1" } }),
            "betterPctRange",
        ),
        (
            json!({ "minSpreadPctRange": { "min": "2", "max": "1" } }),
            "minSpreadPctRange",
        ),
        (json!({ "largeMovePct": "0" }), "largeMovePct"),
        (
            json!({ "farFromAnchorLimitPct": "three" }),
            "farFromAnchorLimitPct",
        ),
        (json!({ "thbCashStep": "0.125" }), "thbCashStep"),
        (json!({ "branchBpCap": 0 }), "branchBpCap"),
        (
            json!({ "marketStaleAfterMinutes": 0 }),
            "marketStaleAfterMinutes",
        ),
        (
            json!({ "rateDecimals": [{ "fromPrice": "1", "decimals": 2 }] }),
            "rateDecimals",
        ),
        (
            json!({ "rateDecimals": [{ "fromPrice": "0", "decimals": 5 }] }),
            "rateDecimals",
        ),
        (
            json!({ "advice": { "farPct": "1", "spreadRatio": "0", "spreadGapPt": "1", "marketMovedPct": "1" } }),
            "advice",
        ),
    ] {
        let answer = room.save("OWNER", body.clone()).await;
        assert_eq!(answer.status, 400, "{body}: {}", answer.text);
        assert_eq!(answer.json()["error"]["code"], "validationFailed", "{body}");
        assert_eq!(field_of(&answer), field, "{body}");
    }
    assert_eq!(room.rows().await, 0, "a refused save writes nothing");
    room.finish().await;
}

/// The two saves really overlap. The owner saves from two sessions at two branches of the one organisation, so
/// the command's per-branch lock does not order them; a third connection holds `pricing_rule_versions` in `SHARE` mode (reads pass, inserts
/// wait) until both saves wait on a lock, then lets go. With the organisation's lock (`lock_in`), the second save
/// waits for the first to commit and builds on it; without it, both read the same "current" and the newer version
/// loses the other's key.
#[actix_web::test]
async fn two_saves_at_once_both_land_in_order() {
    let mut room = Room::open().await;
    room.desk.db.seed_branch_without_cash("B2", "XC-B2").await;
    let pool = room.desk.db.pool.clone();
    let mut conn = pool.acquire().await.expect("acquire connection");
    let owner: String = sqlx::query_scalar(
        "SELECT u.id::text FROM users u \
         WHERE EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id \
                       AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at LIMIT 1",
    )
    .fetch_one(&mut *conn)
    .await
    .expect("the owner");
    let token = xc_testdb::sessions::seed_session(
        &mut conn,
        &owner,
        Some("B2"),
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .await
    .expect("the owner signs in at B2");
    drop(conn);
    room.cookies
        .insert("OWNER@B2".into(), xc_testdb::sessions::cookie(&token));
    let pool = &pool;
    let mut gate = pool.begin().await.expect("open the gate transaction");
    sqlx::query("LOCK TABLE pricing_rule_versions IN SHARE MODE")
        .execute(&mut *gate)
        .await
        .expect("hold inserts back");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("the gate's backend");

    let release = async {
        // Both saves are stuck: behind the gate, or one behind the gate and the other behind the first save.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "WITH RECURSIVE held(pid) AS ( \
                     SELECT $1::int \
                     UNION SELECT a.pid FROM pg_stat_activity a JOIN held h ON h.pid = ANY(pg_blocking_pids(a.pid))) \
                 SELECT count(*) FROM held JOIN pg_stat_activity a USING (pid) \
                 WHERE pid <> $1 AND a.query ILIKE '%pricing_rule_versions%'",
            )
            .bind(gate_pid)
            .fetch_one(pool)
            .await
            .expect("count the waiting saves");
            if waiting >= 2 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "both saves never waited at once ({waiting} waiting)"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        gate.commit().await.expect("open the gate");
    };
    let (a, b, ()) = tokio::join!(
        room.app
            .send(room.save_call("OWNER", json!({ "largeMovePct": "1" }))),
        room.app
            .send(room.save_call("OWNER@B2", json!({ "branchBpCap": 200 }))),
        release,
    );
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(b.status, 200, "{}", b.text);
    assert_eq!(room.rows().await, 2);

    let data = room.view("OWNER").await;
    let rules = &data["current"]["rules"];
    assert_eq!(rules["largeMovePct"], "1", "neither save is lost: {data}");
    assert_eq!(rules["branchBpCap"], 200, "neither save is lost: {data}");
    let history = data["history"].as_array().expect("a history");
    assert_eq!(history.len(), 2);
    assert!(
        history[0]["createdAt"].as_str() > history[1]["createdAt"].as_str(),
        "newest first: {data}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_teller_reads_the_rules() {
    let room = Room::open().await;
    let answer = room.save("OWNER", json!({ "largeMovePct": "1" })).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let data = room.view("FRONT").await;
    assert_eq!(data["current"]["rules"]["largeMovePct"], "1");
    room.finish().await;
}
