//! Issue #137: a branch's snapshot shows its own custody state, never another
//! branch's.
//!
//! `operations` is read for the branch asked about: the custody state, the
//! custody log and the people whose names are joined in. Two branches each get
//! a custody state of their own (another custodian, phase, attendance, request
//! and event) and people with other names, stored through the custody
//! repository the custody service reads back — the same path
//! `custody_json_closed_for_the_night_…` in `snapshot_test.rs` feeds. A third
//! branch has no custody state and must show the initial `operations`.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod compat_common;
mod snapshot_common;

use serde_json::{Value, json};
use snapshot_common::{MAIN, Shop, branch, events_of, state_of, time};
use xc_kernel::ids::BranchId;
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xchange::application::compat::v2_json::{People, PersonLabel};

/// What `operations` is before anything was stored for the branch (the same
/// text `snapshot_test.rs` holds; a test file cannot use another's constants).
const INITIAL_OPERATIONS: &str = concat!(
    r#"{"enabled":false,"phase":"CLOSED","safe":null,"target":null,"entity":null,"custodian":null,"#,
    r#""attendance":[],"requests":[],"events":[],"cutoff":null,"exceptions":[],"holds":[],"drafts":[]}"#
);

/// The custody of one branch, told apart from every other branch's by `tag`:
/// its custodian, the one on duty, one pending request and one event, each
/// with an id of its own.
fn operations(tag: &str, phase: &str, front: &str, rates: &str) -> Value {
    json!({
        "enabled": true,
        "phase": phase,
        "safe": null,
        "target": null,
        "entity": null,
        "custodian": front,
        "attendance": [{
            "id": format!("AT-{tag}"),
            "employeeId": front,
            "role": "FRONT",
            "in": "2026-10-06T05:00:00.000Z",
        }],
        "requests": [{
            "id": format!("RQ-{tag}"),
            "requestId": format!("request-{tag}"),
            "hash": format!("hash-{tag}"),
            "kind": "OPEN",
            "status": "PENDING",
            "at": "2026-10-06T05:01:00.000Z",
            "sender": front,
            "note": format!("note {tag}"),
            "evidenceKeys": [],
            "signatures": [],
        }],
        "events": [{
            "id": format!("EV-{tag}"),
            "type": "CASH_TARGET",
            "at": "2026-10-06T05:02:00.000Z",
            "actor": rates,
            "role": "RATES",
            "details": {"branch": tag},
        }],
        "cutoff": null,
        "exceptions": [],
        "holds": [],
        "drafts": [],
    })
}

/// The people of one branch: the front and the rates employee of its custody.
fn people(front: (&str, &str), rates: (&str, &str, &str)) -> People {
    [
        (
            front.0.to_string(),
            PersonLabel {
                name: front.1.to_string(),
                role: "FRONT".to_string(),
            },
        ),
        (
            rates.0.to_string(),
            PersonLabel {
                name: rates.1.to_string(),
                role: rates.2.to_string(),
            },
        ),
    ]
    .into_iter()
    .collect()
}

/// Stores `ops` as the custody state and log of `here` through the custody
/// repository, as `Replay::feed_custody` does.
async fn store_custody(shop: &Shop, here: &BranchId, ops: &Value) {
    let state = state_of(ops);
    let events = events_of(ops);
    let (repo, b) = (shop.custody_repo.clone(), here.clone());
    shop.db
        .branch_tx()
        .run(here, move |tx| {
            Box::pin(async move {
                let before = repo.load_state_in(tx, &b).await?;
                repo.store_state_in(tx, &b, &before, &state).await?;
                repo.append_events_in(tx, &b, &events).await
            })
        })
        .await
        .expect("store the custody state");
}

/// The part of a printed `operations` that tells one branch from another.
fn seen(operations: &Value) -> Value {
    let rows = |key: &str, fields: &[&str]| -> Value {
        Value::Array(
            operations[key]
                .as_array()
                .unwrap_or_else(|| panic!("`{key}` is not an array in {operations}"))
                .iter()
                .map(|row| Value::Array(fields.iter().map(|field| row[*field].clone()).collect()))
                .collect(),
        )
    };
    json!({
        "enabled": operations["enabled"],
        "phase": operations["phase"],
        "custodian": operations["custodian"],
        "attendance": rows("attendance", &["id", "employeeId", "name"]),
        "requests": rows("requests", &["id", "requestId", "sender", "senderName", "note"]),
        "events": rows("events", &["id", "actor", "role", "details"]),
    })
}

#[tokio::test]
async fn each_branch_shows_its_own_custodian_requests_log_and_people() {
    let shop = Shop::open().await;
    let main = branch(MAIN);
    shop.db.seed_branch("AIRPORT", "Airport").await;
    shop.db.seed_branch("NIGHT", "Night").await;
    let airport = branch("AIRPORT");
    let night = branch("NIGHT");

    store_custody(&shop, &main, &operations("MAIN", "OPEN", "fr1", "ra1")).await;
    store_custody(
        &shop,
        &airport,
        &operations("AIRPORT", "READY", "fr2", "ra2"),
    )
    .await;
    shop.hand.set_people(
        &main,
        people(("fr1", "Main Front"), ("ra1", "Main Rates", "RATES")),
    );
    shop.hand.set_people(
        &airport,
        people(
            ("fr2", "Airport Front"),
            ("ra2", "Airport System", "SYSTEM"),
        ),
    );

    // CA-10 (#626 §6 Q6, ruled on #626): the custody machine is gone, so no branch names a custodian and
    // no custody request rows are read; attendance and events stay per branch.
    let expected_main = json!({
        "enabled": true,
        "phase": "OPEN",
        "custodian": null,
        "attendance": [["AT-MAIN", "fr1", "Main Front"]],
        "requests": [],
        "events": [["EV-MAIN", "ra1", "RATES", {"branch": "MAIN"}]],
    });
    let expected_airport = json!({
        "enabled": true,
        "phase": "READY",
        "custodian": null,
        "attendance": [["AT-AIRPORT", "fr2", "Airport Front"]],
        "requests": [],
        "events": [["EV-AIRPORT", "ra2", "SYSTEM", {"branch": "AIRPORT"}]],
    });
    let initial: Value = serde_json::from_str(INITIAL_OPERATIONS).expect("JSON");

    let now = time("2026-10-06T05:10:00.000Z");
    let builder = shop.builder();
    for (here, expected) in [(&main, &expected_main), (&airport, &expected_airport)] {
        let snapshot = builder
            .admin_snapshot(here, "owner", now)
            .await
            .unwrap_or_else(|e| panic!("snapshot of {}: {e:?}", here.as_str()))
            .to_value();
        let desk = builder
            .desk_config(here)
            .await
            .unwrap_or_else(|e| panic!("desk of {}: {e:?}", here.as_str()))
            .to_value();
        assert_eq!(
            &seen(&snapshot["operations"]),
            expected,
            "adminSnapshot operations of {}",
            here.as_str()
        );
        assert_eq!(
            &seen(&desk["operations"]),
            expected,
            "adminDeskConfig operations of {}",
            here.as_str()
        );
        assert_eq!(
            snapshot["operations"],
            desk["operations"],
            "both answers carry the same operations for {}",
            here.as_str()
        );
    }

    // a branch with no custody state beside two that have one
    let snapshot = builder
        .admin_snapshot(&night, "owner", now)
        .await
        .expect("snapshot of NIGHT")
        .to_value();
    let desk = builder
        .desk_config(&night)
        .await
        .expect("desk of NIGHT")
        .to_value();
    assert_eq!(snapshot["operations"], initial, "adminSnapshot of NIGHT");
    assert_eq!(desk["operations"], initial, "adminDeskConfig of NIGHT");

    shop.finish().await;
}
