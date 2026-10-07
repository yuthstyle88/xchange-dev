//! Tests of the fixture reader `compat_common`, which every T11 test file uses.
//!
//! No database. The fixtures are the contract of `/api/rpc` and `/api/staff/rpc`
//! recorded from v2 by `api/tools/v2-fixtures/gen_compat.mjs`; these tests hold
//! the reader to the files, so that a later test which fails is a failure of the
//! code under test and not of the reading.

mod compat_common;

use compat_common::{
    Fixture, INDEX_FILE, IdMap, Index, RPC_FILES, SNAPSHOT_FILES, Step, expand_shared,
    has_shared_reference, server_ids,
};
use serde_json::{Value, json};

/// Every step file with the number of steps it was recorded with.
const STEP_COUNTS: [(&str, usize); 9] = [
    ("snapshot/empty.json", 48),
    ("snapshot/rates.json", 21),
    ("snapshot/shift.json", 59),
    ("snapshot/transactions.json", 56),
    ("snapshot/custody.json", 255),
    ("snapshot/branches.json", 39),
    ("rpc/transport.json", 99),
    ("rpc/engine_actions.json", 184),
    ("rpc/ops_actions.json", 140),
];

fn step_files() -> Vec<&'static str> {
    SNAPSHOT_FILES
        .iter()
        .chain(RPC_FILES.iter())
        .copied()
        .collect()
}

#[test]
fn all_ten_files_load_with_the_recorded_number_of_steps() {
    assert_eq!(step_files().len(), STEP_COUNTS.len());
    for (name, count) in STEP_COUNTS {
        assert!(
            step_files().contains(&name),
            "{name} is not listed by the reader"
        );
        let fixture = Fixture::load(name);
        assert_eq!(fixture.steps().len(), count, "{name}");
        assert_eq!(fixture.origin, "http://desk.test", "{name}");
        assert_eq!(
            fixture
                .default_headers
                .get("cache-control")
                .map(String::as_str),
            Some("no-store"),
            "{name}"
        );
        assert_eq!(
            fixture
                .default_headers
                .get("content-type")
                .map(String::as_str),
            Some("application/json"),
            "{name}"
        );
        assert!(
            fixture.person("owner").is_some_and(|p| p.is_owner()),
            "{name}"
        );
    }
    let index = Index::load();
    assert_eq!(index.actions.len(), 30, "{INDEX_FILE}");
    // 97 since ticket #146 recorded the owner's ORIGIN_REJECTED
    assert_eq!(index.sentences.len(), 97, "{INDEX_FILE}");
}

#[test]
fn no_reference_is_left_after_expansion() {
    for name in step_files() {
        let fixture = Fixture::load(name);
        for step in fixture.steps() {
            for value in [&step.body, &step.payload].into_iter().flatten() {
                assert!(!has_shared_reference(value), "{name}#{}", step.n);
            }
        }
    }
}

#[test]
fn references_are_expanded_recursively() {
    let shared = json!({
        "s1": {"code": "USD", "history": [{"$shared": "s2"}]},
        "s2": {"buy": 32.1, "inner": {"$shared": "s3"}},
        "s3": [1, 2, 3],
    });
    let value = json!({"rates": [{"$shared": "s1"}, {"code": "EUR"}], "top": {"$shared": "s3"}});
    let expanded = expand_shared(&value, shared.as_object().expect("an object"));
    assert_eq!(
        expanded,
        json!({
            "rates": [
                {"code": "USD", "history": [{"buy": 32.1, "inner": [1, 2, 3]}]},
                {"code": "EUR"},
            ],
            "top": [1, 2, 3],
        })
    );
    assert!(has_shared_reference(&value));
    assert!(!has_shared_reference(&expanded));
}

#[test]
fn the_files_really_use_nested_references() {
    // a reader that expanded one level only would leave a reference behind:
    // shared values of the files hold references themselves
    let path = compat_common::fixtures_dir().join("snapshot/custody.json");
    let raw: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("JSON");
    let shared = raw["shared"].as_object().expect("the shared table");
    assert_eq!(shared.len(), 417);
    let nested = shared.values().filter(|v| has_shared_reference(v)).count();
    assert_eq!(nested, 115);
    let written = raw["steps"].as_array().expect("steps");
    assert!(written.iter().any(|s| has_shared_reference(&s["body"])));
}

#[test]
fn every_step_is_one_of_the_recorded_shapes() {
    for name in step_files() {
        let fixture = Fixture::load(name);
        for step in fixture.steps() {
            let at = format!("{name}#{}", step.n);
            if step.sent_as == "generator" {
                assert!(step.sql.is_some() && !step.is_request(), "{at}");
                assert!(step.status.is_none() && step.body.is_none(), "{at}");
                continue;
            }
            assert!(step.is_request() && step.route.is_some(), "{at}");
            assert!(step.status.is_some(), "{at}");
            assert!(
                step.body.is_some() != step.text.is_some(),
                "{at}: an answer is JSON or text, never both and never neither"
            );
            assert_eq!(step.action.is_some(), step.payload.is_some(), "{at}");
            assert_eq!(step.raw_length.is_some(), step.raw_start.is_some(), "{at}");
            let kinds_of_body = [
                step.action.is_some(),
                step.raw.is_some(),
                step.raw_length.is_some(),
                step.upload.is_some(),
            ];
            assert!(
                kinds_of_body.iter().filter(|k| **k).count() <= 1,
                "{at}: one way of giving the request body"
            );
            assert!(
                step.sent_as == "nobody" || fixture.sender(step).is_some(),
                "{at}: `{}` is not one of the people",
                step.sent_as
            );
            assert!(!fixture.response_headers(step).is_empty(), "{at}");
        }
    }
}

#[test]
fn typed_accessors_read_what_the_files_say() {
    let transport = Fixture::load("rpc/transport.json");

    let sent: &Step = transport.step(1);
    assert_eq!(sent.at, "2026-10-06T05:00:00.000Z");
    assert_eq!(sent.sent_as, "FR1");
    assert_eq!(sent.method.as_deref(), Some("POST"));
    assert_eq!(sent.route.as_deref(), Some("/api/rpc"));
    assert_eq!(
        sent.raw.as_deref(),
        Some(r#"{"action":"getConnectionStatus","payload":{}}"#)
    );
    assert_eq!(sent.status, Some(200));
    assert_eq!(
        sent.json(),
        &json!({"ready": true, "sheetUrl": "/#transactions", "folderUrl": "/documents"})
    );
    assert_eq!(sent.error(), None);
    assert_eq!(transport.response_headers(sent), &transport.default_headers);
    let sender = transport.sender(sent).expect("FR1 is one of the people");
    assert_eq!(
        (sender.role.as_str(), sender.branch_id.as_deref()),
        ("FRONT", Some("MAIN"))
    );
    assert!(!sender.must_change);
    assert!(transport.person("NEW").expect("NEW").must_change);

    // a header taken away, and a refusal
    let no_origin = transport.step(2);
    assert_eq!(no_origin.request_headers.get("origin"), Some(&None));
    assert_eq!(no_origin.status, Some(400));
    assert_eq!(no_origin.error(), Some("คำขอไม่ถูกต้อง"));

    // nobody signed in: the answer has its own headers, without `cache-control`
    let nobody = transport.step(9);
    assert_eq!(nobody.sent_as, "nobody");
    assert!(transport.sender(nobody).is_none());
    assert_eq!(nobody.status, Some(401));
    let headers = transport.response_headers(nobody);
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );

    // an answer that is not JSON
    let get = transport.step(15);
    assert_eq!(get.method.as_deref(), Some("GET"));
    assert_eq!(
        (get.status, get.text.as_deref()),
        (Some(404), Some("Not found"))
    );
    assert!(get.body.is_none() && get.raw.is_none());

    // a body too long to write down
    let long = transport.step(45);
    assert_eq!(long.raw_length, Some(120_000));
    assert!(
        long.raw_start
            .as_deref()
            .is_some_and(|s| s.starts_with(r#"{"action":"#))
    );

    // action and payload, an upload, a row written by the generator, a query string
    let transactions = Fixture::load("snapshot/transactions.json");
    let save_rate = transactions.step(1);
    assert_eq!(save_rate.action.as_deref(), Some("adminSaveRate"));
    assert_eq!(
        save_rate.payload.as_ref().expect("a payload")["code"],
        json!("USD")
    );
    let upload = transactions.step(9);
    assert_eq!(upload.upload.as_deref(), Some("png"));
    assert_eq!(upload.route.as_deref(), Some("/api/upload?branch=MAIN"));
    assert_eq!(upload.path(), Some("/api/upload"));
    let row = transactions.step(24);
    assert_eq!(row.sent_as, "generator");
    assert!(row.sql.as_deref().is_some_and(|s| s.contains("members")));
    assert!(row.note.is_some());
}

#[test]
fn steps_are_found_by_view() {
    let empty = Fixture::load("snapshot/empty.json");
    assert_eq!(empty.views(), vec!["nobody employed yet", "empty"]);
    let first = empty.steps_with_view("nobody employed yet");
    assert!(first.iter().all(|s| s.sent_as == "owner"));
    assert_eq!(first.first().map(|s| s.n), Some(1));
    let all = empty.steps_with_view("empty");
    assert_eq!(
        first.len() + all.len(),
        40,
        "forty of the 48 steps carry a view"
    );
    assert!(
        all.windows(2).all(|pair| pair[0].n < pair[1].n),
        "in the order recorded"
    );
    assert!(
        all.iter()
            .any(|s| s.route.as_deref() == Some("/api/staff/rpc"))
    );
    assert!(empty.steps_with_view("no such view").is_empty());

    let custody = Fixture::load("snapshot/custody.json");
    assert_eq!(custody.views().len(), 9);
    assert_eq!(
        custody.steps().iter().filter(|s| s.view.is_some()).count(),
        174
    );
}

#[test]
fn the_text_of_a_body_is_v2s_own() {
    // Sizes measured on v2 when the fixtures were recorded: the owner's snapshot of
    // an empty shop is 26,973 bytes, the desk configuration 26,761, and what a
    // SYSTEM employee is shown on the staff path 1,295.
    let empty = Fixture::load("snapshot/empty.json");
    for (n, action, route, bytes) in [
        (1, "adminSnapshot", "/api/rpc", 26_973),
        (2, "adminDeskConfig", "/api/rpc", 26_761),
        (31, "adminSnapshot", "/api/staff/rpc", 1_295),
    ] {
        let step = empty.step(n);
        assert_eq!(step.action.as_deref(), Some(action), "empty#{n}");
        assert_eq!(step.route.as_deref(), Some(route), "empty#{n}");
        assert_eq!(
            empty.body_text(n).expect("a body").len(),
            bytes,
            "empty#{n}"
        );
    }
    // keys in the order v2 wrote them, not sorted
    let snapshot = empty.body_text(1).expect("a body");
    assert!(
        snapshot.starts_with(r#"{"version":1,"rateHistory":[],"customers":[],"branchId":"MAIN","#)
    );
    assert!(empty.body_text(15).is_some());

    for name in step_files() {
        let fixture = Fixture::load(name);
        for step in fixture.steps() {
            let at = format!("{name}#{}", step.n);
            let text = fixture.body_text(step.n);
            assert_eq!(text.is_some(), step.body.is_some(), "{at}");
            let Some(text) = text else { continue };
            let parsed: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{at}: {e}"));
            assert_eq!(
                &parsed,
                step.json(),
                "{at}: the text and the value are one answer"
            );
            assert!(!text.contains(r#"{"$shared":"#), "{at}");
            // no spaces: the same bytes as a compact print of the value, whose
            // keys are merely in another order
            assert_eq!(text.len(), parsed.to_string().len(), "{at}");
        }
    }
}

#[test]
fn the_index_reads_per_action_and_route() {
    let index = Index::load();
    assert_eq!(index.claims.get("writeAnswersAreTheSnapshot"), Some(&60));
    assert_eq!(index.claims.get("deskConfigIsPartOfTheSnapshot"), Some(&96));
    assert_eq!(
        index.claims.get("staffPathIsTheSnapshotRewritten"),
        Some(&73)
    );

    let abandon = &index.actions["adminAbandonRequest"];
    assert_eq!(abandon.keys().collect::<Vec<_>>(), vec!["/api/rpc"]);
    let on_rpc = &abandon["/api/rpc"];
    assert_eq!(on_rpc.accepted_from, vec!["OWNER"]);
    let bad_id = on_rpc
        .refusals
        .iter()
        .find(|r| r.error == "รหัสคำขอไม่ถูกต้อง")
        .expect("the refusal of a malformed id");
    assert_eq!(
        (bad_id.status, bad_id.first.as_str()),
        (400, "rpc/engine_actions.json#146")
    );
    assert_eq!(bad_id.from, vec!["OWNER"]);

    // `first` points at a step that says that sentence
    let (file, n) = bad_id.first.split_once('#').expect("file#n");
    let step_n: usize = n.parse().expect("a step number");
    assert_eq!(
        Fixture::load(file).step(step_n).error(),
        Some(bad_id.error.as_str())
    );

    assert!(
        index
            .sentences
            .iter()
            .any(|(s, n)| s == "ใช้ได้เฉพาะ Front" && *n == 53)
    );
}

#[test]
fn server_made_ids_are_found_in_order_of_first_appearance() {
    let value = json!([
        "SH-00000000-0000-4000-8000-000000000011",
        {"file": "/files/00000000-0000-4000-8000-000000000015?download=1"},
        "MV-00000000-0000-4000-8000-000000000013 and SH-00000000-0000-4000-8000-000000000011",
        "CX-00000001",
        "XC-20261006-000001",
        "2026-10-06T05:00:00.000Z",
        "x00000000-0000-4000-8000-000000000099",
        "00000000-0000-4000-8000-0000000000990",
        42,
    ]);
    assert_eq!(
        server_ids(&value),
        vec![
            "SH-00000000-0000-4000-8000-000000000011",
            "00000000-0000-4000-8000-000000000015",
            "MV-00000000-0000-4000-8000-000000000013",
        ]
    );
}

#[test]
fn ids_are_paired_by_kind_and_order_of_first_appearance() {
    let request = "00000000-0000-4000-8000-0000000a0001";
    let recorded = json!({
        "shifts": [
            {"id": "SH-00000000-0000-4000-8000-000000000011", "openRequest": request},
            {"id": "SH-00000000-0000-4000-8000-000000000021", "openRequest": ""},
        ],
        "movements": [{"id": "MV-00000000-0000-4000-8000-000000000013",
                       "shiftId": "SH-00000000-0000-4000-8000-000000000011"}],
        "slipUrl": "/files/00000000-0000-4000-8000-000000000015",
        "customer": "CX-00000001",
    });
    let actual = json!({
        "shifts": [
            {"id": "SH-7d0c2f5e-8a51-4c57-9a43-0c1f6c1b2a01", "openRequest": request},
            {"id": "SH-1b9e7a44-0f0d-4a0e-bb1d-6a5a3a2f9c02", "openRequest": ""},
        ],
        "movements": [{"id": "MV-f3a1c0de-2222-4e7b-8d10-9b8c7d6e5f03",
                       "shiftId": "SH-7d0c2f5e-8a51-4c57-9a43-0c1f6c1b2a01"}],
        "slipUrl": "/files/5e6f7a8b-3333-4c9d-a0b1-c2d3e4f5a604",
        "customer": "CX-00000001",
    });
    assert_ne!(recorded, actual);

    let ids = IdMap::pair(&recorded, &actual);
    assert_eq!(ids.len(), 5);
    assert_eq!(
        ids.actual("SH-00000000-0000-4000-8000-000000000021"),
        Some("SH-1b9e7a44-0f0d-4a0e-bb1d-6a5a3a2f9c02")
    );
    assert_eq!(
        ids.recorded("MV-f3a1c0de-2222-4e7b-8d10-9b8c7d6e5f03"),
        Some("MV-00000000-0000-4000-8000-000000000013")
    );
    assert_eq!(
        ids.actual(request),
        Some(request),
        "an id the caller sent pairs with itself"
    );
    assert_eq!(
        ids.actual("CX-00000001"),
        None,
        "counted ids are not server-made uuids"
    );
    assert_eq!(ids.to_actual(&recorded), actual);

    // a different answer is still different once the ids are mapped
    let mut wrong = actual.clone();
    wrong["movements"][0]["shiftId"] = json!("SH-1b9e7a44-0f0d-4a0e-bb1d-6a5a3a2f9c02");
    assert_ne!(IdMap::pair(&recorded, &wrong).to_actual(&recorded), wrong);
}

#[test]
fn pairs_once_learned_are_kept_across_steps() {
    let mut ids = IdMap::new();
    assert!(ids.is_empty());
    ids.learn(
        &json!({"id": "SH-00000000-0000-4000-8000-000000000011"}),
        &json!({"id": "SH-aaaaaaaa-0000-4000-8000-000000000001"}),
    );
    // the next answer lists the newer shift first on one side only: position
    // alone would cross the two, the pair already known must not move
    let recorded = json!([
        "SH-00000000-0000-4000-8000-000000000011",
        "SH-00000000-0000-4000-8000-000000000021"
    ]);
    let actual = json!([
        "SH-aaaaaaaa-0000-4000-8000-000000000001",
        "SH-bbbbbbbb-0000-4000-8000-000000000002"
    ]);
    ids.learn(&recorded, &actual);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids.to_actual(&recorded), actual);

    // an id present on one side only stays as recorded and shows in the comparison
    let lonely = json!(["MV-00000000-0000-4000-8000-000000000013"]);
    ids.learn(&lonely, &json!([]));
    assert_eq!(ids.len(), 2);
    assert_eq!(ids.to_actual(&lonely), lonely);
}
