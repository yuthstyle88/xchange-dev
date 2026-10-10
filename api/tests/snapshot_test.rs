//! Written before the implementation (ticket T11b). Do not edit, delete or weaken.
//!
//! `SnapshotBuilder`: `adminSnapshot`, `adminDeskConfig` and the answer of the
//! six write actions, against what v2 recorded in `tests/fixtures/snapshot/`.
//!
//! The services of each context are driven directly — no HTTP, no session. A
//! recorded write is applied through its service with the step's `at` as `now`;
//! a recorded read is compared with what the builder answers, once the ids the
//! server makes are paired and the keys v3 leaves out by decision are dropped
//! from v2's answer (`row`, `startRow`, `endRow`, `rawStatus`).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod compat_common;
mod snapshot_common;

use chrono::{TimeZone, Utc};
use compat_common::{Fixture, Step};
use serde_json::{Value, json};
use snapshot_common::{
    MAIN, NO_SUCH_BRANCH, Replay, Shop, WRITE_ACTIONS, assert_same, branch, people_of, refusal,
    time, transaction_of, with_ids, without_dropped_keys,
};
use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_rate::domain::branch::commands::create_branch::CreateBranch;
use xc_rate::domain::rate::commands::save_rate::SaveRate;
use xc_rate::domain::rate::entities::rate_source::ReferenceRates;
use xc_shift::domain::shift::commands::move_stock::MoveStock;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xchange::application::compat::snapshot::write_answer;
use xchange::application::compat::snapshot_sources::{
    MemberMark, NothingYet, SnapshotSources, SnapshotTransaction,
};
use xchange::application::compat::v2_json::V2Json;

const SNAPSHOT_KEYS: [&str; 15] = [
    "version",
    "rateHistory",
    "customers",
    "branchId",
    "branches",
    "transactions",
    "rates",
    "stock",
    "updated",
    "ready",
    "actor",
    "syncedAt",
    "sheetUrl",
    "deskUrl",
    "operations",
];

/// v2's keys in v2's order; `paymentMethods` since the 10-06 bundle (`server.mjs:7047`, row PM-D).
const DESK_KEYS: [&str; 7] = [
    "branchId",
    "branchName",
    "rates",
    "activeShift",
    "adminUrl",
    "operations",
    "paymentMethods",
];

const INITIAL_OPERATIONS: &str = concat!(
    r#"{"enabled":false,"phase":"CLOSED","safe":null,"target":null,"entity":null,"custodian":null,"#,
    r#""attendance":[],"requests":[],"events":[],"cutoff":null,"exceptions":[],"holds":[],"drafts":[]}"#
);

fn keys(printed: &V2Json) -> Vec<&str> {
    match printed {
        V2Json::Object(pairs) => pairs.iter().map(|(k, _)| k.as_str()).collect(),
        other => panic!("not an object: {other:?}"),
    }
}

fn member<'a>(printed: &'a V2Json, key: &str) -> &'a V2Json {
    match printed {
        V2Json::Object(pairs) => pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("no `{key}`")),
        other => panic!("not an object: {other:?}"),
    }
}

fn reference() -> ReferenceRates {
    ReferenceRates {
        source: "https://www.bot.or.th/th/statistics/exchange-rate.html".to_string(),
        fetched_at: "2026-10-06T05:00:00.000Z".to_string(),
        kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
        rows: Vec::new(),
    }
}

fn save_rate(request: u32, code: &str, buy: &str, sell: &str) -> SaveRate {
    SaveRate {
        request_id: format!("00000000-0000-4000-8000-0000000b{request:04}"),
        code: code.to_string(),
        buy: buy.to_string(),
        sell: sell.to_string(),
        partner_name: String::new(),
        partner_cost: None,
        previous_updated: String::new(),
    }
}

fn customer(name: &str, passport: &str, mode: &str) -> ReserveCustomer {
    ReserveCustomer {
        name: name.to_string(),
        passport_no: passport.to_string(),
        nationality: "DEU".to_string(),
        birth_date: "1990-01-02".to_string(),
        passport_expiry: "2031-06-11".to_string(),
        mode: mode.to_string(),
    }
}

/// The steps of `fixture` that one of the six write actions answered with 200.
fn accepted_writes(fixture: &Fixture) -> Vec<&Step> {
    fixture
        .steps()
        .iter()
        .filter(|s| {
            s.status == Some(200)
                && s.action
                    .as_deref()
                    .is_some_and(|a| WRITE_ACTIONS.contains(&a))
        })
        .collect()
}

// ── the empty shop: v2's own text ────────────────────────────────────────────

#[tokio::test]
async fn the_empty_shop_is_answered_with_v2s_text_byte_for_byte() {
    let fixture = Fixture::load("snapshot/empty.json");
    let shop = Shop::open().await;
    let builder = shop.builder();

    // nothing in an empty shop is left out or made by the server, so the whole
    // text — every key, in v2's order, every number as v2 printed it — is equal
    let step = fixture.step(1);
    let snapshot = builder
        .admin_snapshot(&branch(MAIN), "owner", time(&step.at))
        .await
        .expect("the snapshot of MAIN");
    assert_eq!(
        Some(snapshot.to_text()),
        fixture.body_text(1),
        "empty.json#1"
    );
    assert_eq!(keys(&snapshot), SNAPSHOT_KEYS);

    let desk = builder
        .desk_config(&branch(MAIN))
        .await
        .expect("the desk config");
    assert_eq!(Some(desk.to_text()), fixture.body_text(2), "empty.json#2");
    assert_eq!(keys(&desk), DESK_KEYS);

    shop.finish().await;
}

// ── replaying the four files that hold no transaction and no custody ─────────

#[tokio::test]
async fn empty_json_is_reproduced_for_every_caller_of_api_rpc() {
    let fixture = Fixture::load("snapshot/empty.json");
    let mut replay = Replay::start(&fixture).await;
    replay.run().await;

    // 12 snapshots and 11 desk configs were answered on /api/rpc: the owner and
    // nine roles, each with their own username as `actor`
    assert_eq!(replay.compared.snapshots, 12);
    assert_eq!(replay.compared.desks, 11);
    assert_eq!(replay.compared.writes, 0);
    replay.shop.finish().await;
}

#[tokio::test]
async fn rates_json_is_reproduced_step_by_step() {
    let fixture = Fixture::load("snapshot/rates.json");
    let mut replay = Replay::start(&fixture).await;
    replay.run().await;

    assert_eq!(replay.compared.writes, accepted_writes(&fixture).len());
    assert_eq!(replay.compared.writes, 5);
    assert_eq!(replay.compared.snapshots, 5);
    assert_eq!(replay.compared.desks, 5);
    replay.shop.finish().await;
}

#[tokio::test]
async fn shift_json_is_reproduced_step_by_step() {
    let fixture = Fixture::load("snapshot/shift.json");
    let mut replay = Replay::start(&fixture).await;
    replay.run().await;

    assert_eq!(replay.compared.writes, accepted_writes(&fixture).len());
    assert_eq!(replay.compared.writes, 7);
    assert_eq!(replay.compared.snapshots, 15);
    assert_eq!(replay.compared.desks, 15);
    // two shifts and three movements were made by the server and paired
    assert!(replay.ids.len() >= 5, "ids paired: {}", replay.ids.len());
    replay.shop.finish().await;
}

#[tokio::test]
async fn branches_json_is_reproduced_step_by_step() {
    let fixture = Fixture::load("snapshot/branches.json");
    let mut replay = Replay::start(&fixture).await;
    replay.run().await;

    assert_eq!(replay.compared.writes, accepted_writes(&fixture).len());
    assert_eq!(replay.compared.writes, 11);
    assert_eq!(replay.compared.snapshots, 8);
    assert_eq!(replay.compared.desks, 7);
    assert_eq!(replay.compared.reservations, 2);
    replay.shop.finish().await;
}

#[tokio::test]
async fn an_open_shift_is_the_desks_active_shift_and_a_closed_one_is_not() {
    let fixture = Fixture::load("snapshot/shift.json");
    let mut replay = Replay::start(&fixture).await;

    // #2 opens the first shift
    replay.run_range(1, 2).await;
    let desk = replay.shop.builder().desk_config(&branch(MAIN)).await;
    let desk = desk.expect("desk").to_value();
    let open = replay.shop.shifts.active(&branch(MAIN)).await;
    let open = open.expect("active").expect("a shift is open");
    assert_eq!(desk["activeShift"], json!(open.id));

    // #37 closes it
    replay.run_range(3, 37).await;
    let desk = replay.shop.builder().desk_config(&branch(MAIN)).await;
    assert_eq!(desk.expect("desk").to_value()["activeShift"], Value::Null);
    replay.shop.finish().await;
}

// ── refusals and the decided difference ──────────────────────────────────────

#[tokio::test]
async fn a_branch_that_does_not_exist_is_refused_with_v2s_sentence() {
    let fixture = Fixture::load("snapshot/empty.json");
    let shop = Shop::open().await;
    let builder = shop.builder();
    let nowhere = branch("B-nowhere");

    // empty.json#44 and #45
    assert_eq!(fixture.step(44).error(), Some(NO_SUCH_BRANCH));
    assert_eq!(fixture.step(45).error(), Some(NO_SUCH_BRANCH));
    let now = time(&fixture.step(44).at);
    assert_eq!(
        refusal(builder.admin_snapshot(&nowhere, "owner", now).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(refusal(builder.desk_config(&nowhere).await), NO_SUCH_BRANCH);
    shop.finish().await;
}

#[tokio::test]
async fn create_branch_sent_again_from_main_answers_the_new_branchs_snapshot() {
    // branches.json#3: v2 refuses this replay with a sentence about table
    // headers. Decided difference: v3 answers as it does for #4.
    let fixture = Fixture::load("snapshot/branches.json");
    let mut replay = Replay::start(&fixture).await;
    replay.run_range(1, 2).await;

    let again = fixture.step(3);
    assert_eq!(again.status, Some(400), "v2 refused it");
    let payload = again.payload.as_ref().expect("a payload");
    let created = replay
        .shop
        .branches
        .create(
            CreateBranch {
                request_id: payload["requestId"].as_str().expect("id").to_string(),
                name: payload["name"].as_str().expect("name").to_string(),
                from_branch: branch(MAIN),
            },
            "owner",
            time(&again.at),
        )
        .await
        .expect("the same request again is the same branch");
    assert_eq!(created.id.as_str(), "B00000000-0000-4000-8000-0000000a0002");

    // what v2 answered when the same request came from the new branch (#4),
    // at the clock of #3
    let as_fourth = fixture.step(4);
    let answer = replay
        .shop
        .builder()
        .admin_snapshot(&created.id, "owner", time(&as_fourth.at))
        .await
        .expect("the new branch's snapshot");
    assert_same(
        as_fourth.json(),
        &write_answer(&answer).to_value(),
        "branches.json#4",
    );
    replay.shop.finish().await;
}

// ── write_answer ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_write_answer_is_the_snapshot_without_operations_and_member_fields() {
    let shop = Shop::open().await;
    shop.customers
        .reserve(
            &branch(MAIN),
            customer("Customer 1", "P1000001", "LIVE"),
            time("2026-10-06T05:09:00.000Z"),
        )
        .await
        .expect("reserve");
    shop.hand.add_member(
        &branch(MAIN),
        MemberMark {
            customer_id: "CX-00000001".to_string(),
            verified_at: Some(time("2026-10-06T04:00:00.000Z")),
        },
    );
    let snapshot = shop
        .builder()
        .admin_snapshot(&branch(MAIN), "owner", time("2026-10-06T05:10:00.000Z"))
        .await
        .expect("snapshot");
    let answer = write_answer(&snapshot);

    // every key but the last, in the same order
    assert_eq!(keys(&answer), SNAPSHOT_KEYS[..14]);
    // the customer is still there, a member or not, without the two fields
    assert_eq!(
        member(&answer, "customers").to_text(),
        r#"[{"id":"CX-00000001","name":"Customer 1","nationality":"DEU","created":"2026-10-06T05:09:00.000Z"}]"#
    );
    // nothing else changed
    for key in SNAPSHOT_KEYS
        .iter()
        .filter(|k| **k != "customers" && **k != "operations")
    {
        assert_eq!(member(&answer, key), member(&snapshot, key), "{key}");
    }
    // and the snapshot itself was not touched
    assert_eq!(
        member(&snapshot, "customers").to_text(),
        concat!(
            r#"[{"id":"CX-00000001","name":"Customer 1","nationality":"DEU","created":"2026-10-06T05:09:00.000Z","#,
            r#""member":true,"memberVerifiedAt":"2026-10-06T04:00:00.000Z"}]"#
        )
    );
    shop.finish().await;
}

// ── what the caller and the clock decide ─────────────────────────────────────

#[tokio::test]
async fn actor_is_the_caller_and_synced_at_is_the_clock_cut_to_milliseconds() {
    let shop = Shop::open().await;
    let clock = Utc
        .timestamp_opt(1_791_288_000, 123_456_789)
        .single()
        .expect("an instant");
    let snapshot = shop
        .builder()
        .admin_snapshot(&branch(MAIN), "fr1", clock)
        .await
        .expect("snapshot")
        .to_value();
    assert_eq!(snapshot["actor"], "fr1");
    assert_eq!(snapshot["syncedAt"], "2026-10-06T12:00:00.123Z");
    assert_eq!(snapshot["version"], 1);
    assert_eq!(snapshot["ready"], true);
    assert_eq!(snapshot["sheetUrl"], "/#transactions");
    // no rate was ever saved here
    assert_eq!(snapshot["updated"], "");
    shop.finish().await;
}

#[tokio::test]
async fn updated_is_the_newest_rate_row_of_the_branch_whatever_its_currency() {
    let shop = Shop::open().await;
    shop.source.answer_next(reference());
    for (n, code, at) in [
        (1, "EUR", "2026-10-06T05:01:00.000Z"),
        (2, "USD", "2026-10-06T05:02:00.000Z"),
        (3, "JPY", "2026-10-06T05:03:00.000Z"),
    ] {
        shop.rates
            .save_rate(
                &branch(MAIN),
                save_rate(n, code, "1.5", "2.5"),
                "owner",
                time(at),
            )
            .await
            .expect("save a rate");
    }
    let snapshot = shop
        .builder()
        .admin_snapshot(&branch(MAIN), "owner", time("2026-10-06T06:00:00.000Z"))
        .await
        .expect("snapshot")
        .to_value();
    // JPY is neither first nor last in register order: `updated` is not read
    // off the rate list
    assert_eq!(snapshot["updated"], "2026-10-06T05:03:00.000Z");
    assert_eq!(snapshot["rateHistory"][0]["code"], "JPY");
    assert_eq!(snapshot["rateHistory"].as_array().map(Vec::len), Some(3));
    assert_eq!(snapshot["rates"].as_array().map(Vec::len), Some(152));
    shop.finish().await;
}

#[tokio::test]
async fn the_two_links_carry_the_branch_as_encode_uri_component_writes_it() {
    let fixture = Fixture::load("snapshot/branches.json");
    let shop = Shop::open().await;
    // no service makes such an id; the links must still be safe to follow
    shop.db.seed_branch("a b/ü&c", "Odd").await;
    let odd = branch("a b/ü&c");
    let snapshot = shop
        .builder()
        .admin_snapshot(&odd, "owner", time("2026-10-06T05:00:00.000Z"))
        .await
        .expect("snapshot")
        .to_value();
    assert_eq!(snapshot["branchId"], "a b/ü&c");
    assert_eq!(snapshot["deskUrl"], "/desk?branch=a%20b%2F%C3%BC%26c");
    let desk = shop.builder().desk_config(&odd).await.expect("desk");
    let desk = desk.to_value();
    assert_eq!(desk["adminUrl"], "/?branch=a%20b%2F%C3%BC%26c");
    assert_eq!(desk["branchName"], "Odd");

    // and an ordinary id is written as it is (branches.json#13, #14)
    assert_eq!(
        fixture.step(13).json()["deskUrl"],
        "/desk?branch=B00000000-0000-4000-8000-0000000a0002"
    );
    assert_eq!(
        fixture.step(14).json()["adminUrl"],
        "/?branch=B00000000-0000-4000-8000-0000000a0002"
    );
    shop.finish().await;
}

// ── two branches ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn two_branches_do_not_see_each_others_rates_shifts_and_customers() {
    let shop = Shop::open().await;
    let main = branch(MAIN);
    let airport = shop
        .branches
        .create(
            CreateBranch {
                request_id: "00000000-0000-4000-8000-0000000b0001".to_string(),
                name: "Airport".to_string(),
                from_branch: main.clone(),
            },
            "owner",
            time("2026-10-06T05:00:00.000Z"),
        )
        .await
        .expect("create")
        .id;

    shop.source.answer_next(reference());
    shop.rates
        .save_rate(
            &main,
            save_rate(2, "USD", "32.1", "32.6"),
            "owner",
            time("2026-10-06T05:01:00.000Z"),
        )
        .await
        .expect("a rate at MAIN");
    shop.rates
        .save_rate(
            &airport,
            save_rate(3, "EUR", "37", "39"),
            "owner",
            time("2026-10-06T05:02:00.000Z"),
        )
        .await
        .expect("a rate at the airport");
    let opening =
        json!({"requestId": "00000000-0000-4000-8000-0000000b0004", "opening": {"THB": 20000}});
    snapshot_common::seed_opener(&shop.db.pool, "be2").await;
    let shift = shop
        .shifts
        .open(
            &airport,
            OpenShift::from_payload(&opening).expect("opening"),
            "be2",
            time("2026-10-06T05:03:00.000Z"),
        )
        .await
        .expect("a shift at the airport");
    let moved = json!({
        "requestId": "00000000-0000-4000-8000-0000000b0005", "shiftId": shift.id,
        "currency": "THB", "direction": "IN", "amount": 500, "reason": "from the safe",
    });
    shop.shifts
        .move_stock(
            &airport,
            MoveStock::from_payload(&moved).expect("movement"),
            "fi2",
            time("2026-10-06T05:03:30.000Z"),
        )
        .await
        .expect("a movement at the airport");
    shop.customers
        .reserve(
            &main,
            customer("At Main", "P1", "LIVE"),
            time("2026-10-06T05:04:00.000Z"),
        )
        .await
        .expect("a customer at MAIN");
    shop.customers
        .reserve(
            &airport,
            customer("At Airport", "P2", "LIVE"),
            time("2026-10-06T05:05:00.000Z"),
        )
        .await
        .expect("a customer at the airport");
    shop.hand.add_transaction(
        &airport,
        transaction_of(
            &json!({
                "id": "XC-20261006-000001", "status": "COMPLETED", "created": "2026-10-06T05:06:00.000Z",
                "date": "2026-10-06", "type": "BUY", "currency": "EUR", "amount": 10, "rate": 37,
                "referenceRate": null, "fee": 0, "bankFee": 0, "slipUrl": null, "paymentRef": "",
                "customerId": "CX-00000002", "customer": "At Airport", "staff": "fr2", "notes": "",
                "shiftId": shift.id,
            }),
            "Cash",
        ),
    );
    shop.hand.add_member(
        &airport,
        MemberMark {
            customer_id: "CX-00000001".to_string(), // the id of MAIN's customer
            verified_at: None,
        },
    );

    let now = time("2026-10-06T05:10:00.000Z");
    let builder = shop.builder();
    let at_main = builder.admin_snapshot(&main, "owner", now).await;
    let at_main = at_main.expect("MAIN").to_value();
    let at_airport = builder.admin_snapshot(&airport, "owner", now).await;
    let at_airport = at_airport.expect("airport").to_value();

    let set = |snapshot: &Value| -> Vec<String> {
        snapshot["rates"]
            .as_array()
            .expect("rates")
            .iter()
            .filter(|r| !r["buy"].is_null())
            .map(|r| r["code"].as_str().expect("code").to_string())
            .collect()
    };
    let names = |snapshot: &Value| -> Vec<String> {
        snapshot["customers"]
            .as_array()
            .expect("customers")
            .iter()
            .map(|c| c["name"].as_str().expect("name").to_string())
            .collect()
    };
    assert_eq!(set(&at_main), ["USD"]);
    assert_eq!(set(&at_airport), ["EUR"]);
    assert_eq!(at_main["rateHistory"].as_array().map(Vec::len), Some(1));
    assert_eq!(at_main["rateHistory"][0]["code"], "USD");
    assert_eq!(at_airport["rateHistory"][0]["code"], "EUR");
    assert_eq!(at_main["updated"], "2026-10-06T05:01:00.000Z");
    assert_eq!(at_airport["updated"], "2026-10-06T05:02:00.000Z");
    assert_eq!(at_main["stock"]["shifts"], json!([]));
    assert_eq!(at_airport["stock"]["shifts"][0]["id"], json!(shift.id));
    assert_eq!(at_main["stock"]["movements"], json!([]));
    assert_eq!(
        at_airport["stock"]["movements"][0]["reason"],
        "from the safe"
    );
    // CI-4 (#651): the snapshot lists every LIVE customer of the company (list_live company-wide, owner
    // 2026-10-09, (c)); rates, shifts and transactions stay the branch's.
    assert_eq!(names(&at_main), ["At Main", "At Airport"]);
    assert_eq!(names(&at_airport), ["At Main", "At Airport"]);
    assert_eq!(at_main["transactions"], json!([]));
    assert_eq!(at_airport["transactions"][0]["id"], "XC-20261006-000001");
    // a member mark of another branch marks nobody here
    assert_eq!(at_main["customers"][0]["member"], false);
    // both see the same list of branches, MAIN first
    assert_eq!(at_main["branches"], at_airport["branches"]);
    assert_eq!(
        at_main["branches"][0],
        json!({"id": "MAIN", "name": "XC-B"})
    );
    assert_eq!(at_main["branches"].as_array().map(Vec::len), Some(2));

    let desk_main = builder.desk_config(&main).await.expect("desk").to_value();
    let desk_airport = builder
        .desk_config(&airport)
        .await
        .expect("desk")
        .to_value();
    assert_eq!(desk_main["activeShift"], Value::Null);
    assert_eq!(desk_airport["activeShift"], json!(shift.id));
    assert_eq!(desk_main["branchName"], "XC-B");
    assert_eq!(desk_airport["branchName"], "Airport");
    assert_eq!(desk_airport["rates"], at_airport["rates"]);
    shop.finish().await;
}

// ── the sources: nothing yet, and hand-made ──────────────────────────────────

#[tokio::test]
async fn with_nothing_yet_there_is_no_transaction_no_member_and_nobody_is_named() {
    let shop = Shop::open().await;
    let main = branch(MAIN);
    shop.customers
        .reserve(
            &main,
            customer("Customer 1", "P1000001", "LIVE"),
            time("2026-10-06T05:09:00.000Z"),
        )
        .await
        .expect("reserve");
    // the hand-made sources of this shop hold both; the stub is asked instead
    shop.hand.add_member(
        &main,
        MemberMark {
            customer_id: "CX-00000001".to_string(),
            verified_at: None,
        },
    );
    let snapshot = shop
        .builder_with_nothing_yet()
        .admin_snapshot(&main, "owner", time("2026-10-06T05:10:00.000Z"))
        .await
        .expect("snapshot");
    assert_eq!(member(&snapshot, "transactions").to_text(), "[]");
    assert_eq!(
        member(&snapshot, "customers").to_text(),
        concat!(
            r#"[{"id":"CX-00000001","name":"Customer 1","nationality":"DEU","created":"2026-10-06T05:09:00.000Z","#,
            r#""member":false,"memberVerifiedAt":null}]"#
        )
    );
    assert_eq!(
        member(&snapshot, "operations").to_text(),
        INITIAL_OPERATIONS
    );

    // the stub itself
    assert_eq!(
        NothingYet.transactions(&main).await.expect("ok"),
        Vec::<SnapshotTransaction>::new()
    );
    assert_eq!(
        NothingYet.members(&main).await.expect("ok"),
        Vec::<MemberMark>::new()
    );
    assert!(NothingYet.people(&main).await.expect("ok").is_empty());
    shop.finish().await;
}

#[tokio::test]
async fn a_member_without_a_verification_time_is_a_member_with_null() {
    let shop = Shop::open().await;
    let main = branch(MAIN);
    for (name, passport) in [("Customer 1", "P1"), ("Customer 2", "P2")] {
        shop.customers
            .reserve(
                &main,
                customer(name, passport, "LIVE"),
                time("2026-10-06T05:09:00.000Z"),
            )
            .await
            .expect("reserve");
    }
    shop.hand.add_member(
        &main,
        MemberMark {
            customer_id: "CX-00000002".to_string(),
            verified_at: None,
        },
    );
    let snapshot = shop
        .builder()
        .admin_snapshot(&main, "owner", time("2026-10-06T05:10:00.000Z"))
        .await
        .expect("snapshot")
        .to_value();
    assert_eq!(snapshot["customers"][0]["member"], false);
    assert_eq!(snapshot["customers"][1]["member"], true);
    assert_eq!(snapshot["customers"][1]["memberVerifiedAt"], Value::Null);
    shop.finish().await;
}

#[tokio::test]
async fn a_transaction_is_printed_field_for_field_in_v2s_order() {
    let shop = Shop::open().await;
    let main = branch(MAIN);
    // every field different from every other, and an evening in UTC that is
    // already the next day in Bangkok: `date` is the value date, not the clock's
    shop.hand.add_transaction(
        &main,
        SnapshotTransaction {
            document_ref: "XC-20261007-000009".to_string(),
            recorded_at: time("2026-10-06T18:30:00.123Z"),
            value_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 7).expect("a day"),
            side: xc_kernel::money::Side::Sell,
            currency: "JPY".to_string(),
            amount: xc_kernel::money::Money::parse("1000.50").expect("money"),
            rate: xc_kernel::money::Rate::parse("0.2105").expect("rate"),
            reference_rate: Some(xc_kernel::money::Rate::parse("0.2199").expect("rate")),
            fee: xc_kernel::money::Money::parse("25.50").expect("money"),
            bank_fee: xc_kernel::money::Money::parse("5.00").expect("money"),
            method: "PromptPay".to_string(),
            slip_key: Some("00000000-0000-4000-8000-00000000001c".to_string()),
            payment_ref: "TRF-0001".to_string(),
            customer_id: "CX-00000003".to_string(),
            customer_name: "=A name with a formula sign".to_string(),
            staff: "fr1".to_string(),
            notes: "a \"quoted\" note".to_string(),
            shift_id: Some("SH-1".to_string()),
        },
    );
    shop.hand.add_transaction(
        &main,
        transaction_of(
            &json!({
                "id": "XC-20261007-000010", "status": "COMPLETED", "created": "2026-10-06T18:31:00.000Z",
                "date": "2026-10-07", "type": "BUY", "currency": "USD", "amount": 100, "rate": 32.1,
                "referenceRate": null, "fee": 0, "bankFee": 0, "slipUrl": null, "paymentRef": "",
                "customerId": "TEST-CX-1", "customer": "Demo", "staff": "owner", "notes": "",
                "shiftId": null,
            }),
            "Cash",
        ),
    );
    let snapshot = shop
        .builder()
        .admin_snapshot(&main, "owner", time("2026-10-06T19:00:00.000Z"))
        .await
        .expect("snapshot");
    assert_eq!(
        member(&snapshot, "transactions").to_text(),
        concat!(
            r#"[{"id":"XC-20261007-000009","status":"COMPLETED","created":"2026-10-06T18:30:00.123Z","#,
            r#""date":"2026-10-07","type":"SELL","currency":"JPY","amount":1000.5,"rate":0.2105,"#,
            r#""referenceRate":0.2199,"fee":25.5,"bankFee":5,"method":"PromptPay","#,
            r#""slipUrl":"/files/00000000-0000-4000-8000-00000000001c","paymentRef":"TRF-0001","#,
            r#""customerId":"CX-00000003","customer":"=A name with a formula sign","staff":"fr1","#,
            r#""notes":"a \"quoted\" note","shiftId":"SH-1"},"#,
            // the order the source gave, and what is absent is null
            r#"{"id":"XC-20261007-000010","status":"COMPLETED","created":"2026-10-06T18:31:00.000Z","#,
            r#""date":"2026-10-07","type":"BUY","currency":"USD","amount":100,"rate":32.1,"#,
            r#""referenceRate":null,"fee":0,"bankFee":0,"method":"Cash","slipUrl":null,"paymentRef":"","#,
            r#""customerId":"TEST-CX-1","customer":"Demo","staff":"owner","notes":"","shiftId":null}]"#
        )
    );
    // a write answers with the same list
    assert_eq!(
        member(&write_answer(&snapshot), "transactions"),
        member(&snapshot, "transactions")
    );
    shop.finish().await;
}

#[tokio::test]
async fn transactions_json_is_reproduced_from_exchanges_the_test_feeds() {
    let fixture = Fixture::load("snapshot/transactions.json");
    let last = fixture
        .steps_with_view("five saves, four of them LIVE, and a member")
        .into_iter()
        .find(|s| s.sent_as == "owner" && s.action.as_deref() == Some("adminSnapshot"))
        .expect("the owner's last view");
    let mut replay = Replay::start(&fixture).await;
    // rates, the shift and the customers go through their services; a save is
    // not on main, so each `saveTransaction` step lists its exchange by hand
    replay.recorded_transactions = last.json()["transactions"]
        .as_array()
        .expect("transactions")
        .clone();
    replay.run().await;

    assert_eq!(replay.compared.writes, 3);
    assert_eq!(replay.compared.reservations, 5);
    // five saves, one of them DEMO: no view shows it, nothing lists it
    assert_eq!(replay.compared.saves_fed, 4);
    // "one transaction" for the owner, then the last view for the owner and six roles
    assert_eq!(replay.compared.snapshots, 8);
    assert_eq!(replay.compared.desks, 8);

    let main = branch(MAIN);
    let snapshot = replay
        .shop
        .builder()
        .admin_snapshot(&main, "owner", time(&last.at))
        .await
        .expect("snapshot");

    // the old method name is read back as v2 read it
    let shown = snapshot.to_value();
    let methods: Vec<&str> = shown["transactions"]
        .as_array()
        .expect("transactions")
        .iter()
        .map(|t| t["method"].as_str().expect("method"))
        .collect();
    assert_eq!(methods, ["Cash", "Cash", "Bank Transfer", "Bank Transfer"]);
    let sent = fixture.step(21).payload.as_ref().expect("payload");
    assert_eq!(sent["method"], "SCB Transfer", "transactions.json#21");

    // the DEMO customer and the DEMO exchange are in no list
    assert!(!snapshot.to_text().contains("TEST-"));
    // the member, with the time v2 printed
    assert_eq!(shown["customers"][1]["member"], true);
    assert_eq!(
        shown["customers"][1]["memberVerifiedAt"],
        "2026-10-06T04:00:00.000Z"
    );

    // key for key in v2's order: v2's text of the list, without the two keys
    // v3 leaves out, holds what the builder printed
    let mut v2_text = fixture.body_text(last.n).expect("a view is JSON");
    v2_text = v2_text.replace(r#""rawStatus":"SAVED","#, "");
    for row in 2..=5 {
        v2_text = v2_text.replace(&format!(r#","row":{row}}}"#), "}");
    }
    let recorded_shift = last.json()["stock"]["shifts"][0]["id"]
        .as_str()
        .expect("id");
    let actual_shift = replay
        .ids
        .actual(recorded_shift)
        .expect("the shift was paired");
    v2_text = v2_text.replace(recorded_shift, actual_shift);
    let printed = format!(
        r#""transactions":{}"#,
        member(&snapshot, "transactions").to_text()
    );
    assert!(v2_text.contains(&printed), "not v2's text: {printed}");
    replay.shop.finish().await;
}

#[tokio::test]
async fn custody_json_closed_for_the_night_is_reproduced_from_what_the_test_feeds() {
    let fixture = Fixture::load("snapshot/custody.json");
    let views = fixture.steps_with_view("closed for the night");
    let last = *views
        .iter()
        .find(|s| s.sent_as == "owner" && s.action.as_deref() == Some("adminSnapshot"))
        .expect("the owner's last view");
    let main = branch(MAIN);
    let mut replay = Replay::start(&fixture).await;

    // rates and customers: their recorded steps, through their services
    for step in &fixture.steps()[..last.n - 1] {
        let replayed = matches!(
            step.action.as_deref(),
            Some("adminSaveRate" | "reserveCustomer")
        );
        if replayed && step.status == Some(200) {
            replay.step(step).await;
        }
    }
    assert_eq!(replay.compared.writes, 2);
    assert_eq!(replay.compared.reservations, 3);

    // shifts and movements were made by custody commits, exchanges by saves:
    // neither can be replayed here, so the test makes what the view shows
    let made = replay.feed_stock(&main, last.json()).await;
    assert_eq!(made.len(), 2);
    let view = with_ids(last.json(), &made);
    for recorded in view["transactions"].as_array().expect("transactions") {
        let method = recorded["method"].as_str().expect("method");
        replay
            .shop
            .hand
            .add_transaction(&main, transaction_of(recorded, method));
    }
    replay.feed_custody(&main, &view["operations"]).await;
    replay.shop.hand.set_people(&main, people_of(&fixture));

    // the state, as the owner and every role reads it on /api/rpc
    for step in &views {
        replay.step(step).await;
    }
    // the owner and twelve employees
    assert_eq!(replay.compared.snapshots, 13);
    assert_eq!(replay.compared.desks, 13);

    // what the day left: both shifts closed, three exchanges, fifty-five events
    let snapshot = replay
        .shop
        .builder()
        .admin_snapshot(&main, "owner", time(&last.at))
        .await
        .expect("snapshot")
        .to_value();
    assert_same(
        &with_ids(&without_dropped_keys(last.json()), &made)["stock"]["shifts"],
        &snapshot["stock"]["shifts"],
        "the shifts",
    );
    assert_eq!(snapshot["transactions"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        snapshot["operations"]["events"].as_array().map(Vec::len),
        Some(55)
    );
    assert_eq!(snapshot["operations"]["attendance"][0]["name"], "FR1 Front");
    assert_eq!(snapshot["operations"]["enabled"], true);
    replay.shop.finish().await;
}
