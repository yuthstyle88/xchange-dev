//! XC-V9-PRICING PV-k (pricing-tiers.md §15.1 "effective mode at a branch", §15.5; `H:` §7 last bullet): a branch
//! that is not enrolled in MAIN's FOLLOW shows what it publishes, never `FOLLOW`. Added by the sub-controller
//! claude/451f0116 with the reviewer fix on PR 5191. Do not edit, delete or weaken; add tests in a file of your own.

mod v9_follow_common;

use serde_json::{Value, json};
use v9_follow_common::*;

const BR_B: &str = "BR-B";
const BR_E: &str = "BR-E";

async fn board(room: &Room, branch: &str) -> Value {
    view(room, &owner(), "pricing_board", branch, None)
        .await
        .ok("pricing_board")["data"]
        .clone()
}

fn mode(data: &Value, code: &str) -> Value {
    data["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("rows: {data}"))
        .iter()
        .find(|r| r["currency"] == json!(code))
        .unwrap_or_else(|| panic!("a {code} row"))["mode"]
        .clone()
}

/// PV-k: USD follows at MAIN; B enrols, E does not. B's USD reads FOLLOW; E's reads AUTO (it publishes no USD tier
/// set), not FOLLOW.
#[tokio::test]
async fn only_an_enrolled_branch_reads_follow() {
    let room = Room::start().await;
    let now = whole_second_now();
    for b in [BR_B, BR_E] {
        room.create_branch(b).await;
        room.policy(b).await;
    }
    follow_usd_at_main(&room, now, 3).await;
    enrol(&room, BR_B, "USD", 0, 0).await;
    pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()));

    assert_eq!(mode(&board(&room, MAIN).await, "USD"), json!("FOLLOW"));
    assert_eq!(mode(&board(&room, BR_B).await, "USD"), json!("FOLLOW"));
    assert_eq!(
        mode(&board(&room, BR_E).await, "USD"),
        json!("AUTO"),
        "E is not enrolled (H: §7: a branch default never enrols)"
    );
    room.finish().await;
}
