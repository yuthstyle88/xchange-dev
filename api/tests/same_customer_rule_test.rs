//! CI-4 (#651): the replay rule `SameCustomerAs` (compat_common::SAME_CUSTOMER_AS) cannot become a blanket pass.
//! Lane controller's ruling (A), 2026-10-09: narrow (only `customerId` and `existing` move), strict (a wrong
//! customer or `existing: false` goes red), documented (a reason per use), limited (exactly three uses).
//!
//! Written by the ticket author (claude/89458e53) with CI-4's up-front commit. Do not edit, delete or weaken.

mod compat_common;
mod save_common;
mod snapshot_common;

use compat_common::{IdMap, SAME_CUSTOMER_AS, same_customer_answer, same_customer_as};
use serde_json::json;
use snapshot_common::assert_same;

#[test]
fn exactly_three_steps_use_it_each_with_its_cause() {
    let uses: Vec<(&str, usize, usize, &str)> = SAME_CUSTOMER_AS
        .iter()
        .map(|s| (s.fixture, s.step, s.same_as, s.reason))
        .collect();
    assert_eq!(
        uses,
        vec![
            (
                "rpc/engine_actions.json",
                124,
                77,
                "owner 2026-10-09: DEMO shares LIVE"
            ),
            (
                "snapshot/transactions.json",
                8,
                4,
                "owner 2026-10-09: DEMO shares LIVE"
            ),
            ("snapshot/branches.json", 10, 9, "K1 (customer per company)"),
        ]
    );
    assert!(same_customer_as("rpc/engine_actions.json", 78).is_none());
}

#[test]
fn the_earlier_customer_and_existing_true_pass() {
    let recorded = json!({"customerId": "TEST-CX-00000001", "existing": false});
    let expected = same_customer_answer(&recorded, "CX-00000001", "self-test");
    assert_same(
        &expected,
        &json!({"customerId": "CX-00000001", "existing": true}),
        "self-test",
    );
}

#[test]
#[should_panic(expected = "self-test")]
fn a_different_customer_goes_red() {
    let recorded = json!({"customerId": "TEST-CX-00000001", "existing": false});
    let expected = same_customer_answer(&recorded, "CX-00000001", "self-test");
    assert_same(
        &expected,
        &json!({"customerId": "CX-00000002", "existing": true}),
        "self-test",
    );
}

#[test]
#[should_panic(expected = "self-test")]
fn existing_false_goes_red() {
    let recorded = json!({"customerId": "TEST-CX-00000001", "existing": false});
    let expected = same_customer_answer(&recorded, "CX-00000001", "self-test");
    assert_same(
        &expected,
        &json!({"customerId": "CX-00000001", "existing": false}),
        "self-test",
    );
}

#[test]
#[should_panic(expected = "applies only where v2 made a new customer")]
fn it_never_applies_to_a_recorded_existing_customer() {
    let recorded = json!({"customerId": "CX-00000001", "existing": true});
    let _ = same_customer_answer(&recorded, "CX-00000001", "self-test");
}

#[test]
fn the_alias_replaces_only_that_whole_customer_id() {
    let mut ids = IdMap::new();
    ids.alias_customer("TEST-CX-00000001", "CX-00000001");
    assert_eq!(
        ids.to_actual(
            &json!({"customerId": "TEST-CX-00000001", "other": "TEST-CX-00000002",
            "note": "TEST-CX-00000001 inside text"})
        ),
        json!({"customerId": "CX-00000001", "other": "TEST-CX-00000002",
            "note": "TEST-CX-00000001 inside text"})
    );
}

#[test]
fn a_receipt_url_that_is_the_stored_receipts_hash_passes() {
    use sha2::{Digest, Sha256};
    let receipt = b"<html>CX-00000001</html>";
    let url = format!("/files/receipt-{:x}", Sha256::digest(receipt));
    let recorded = json!({"documentRef": "XC-1", "receiptUrl": "/files/receipt-recorded"});
    let expected = compat_common::same_customer_save_answer(&recorded, &url, receipt, "self-test");
    assert_eq!(expected, json!({"documentRef": "XC-1", "receiptUrl": url}));
}

#[test]
#[should_panic(expected = "the receipt URL is not the hash of the stored receipt")]
fn a_receipt_url_that_is_not_the_stored_receipts_hash_goes_red() {
    let recorded = json!({"documentRef": "XC-1", "receiptUrl": "/files/receipt-recorded"});
    let _ = compat_common::same_customer_save_answer(
        &recorded,
        "/files/receipt-0000",
        b"<html>CX-00000001</html>",
        "self-test",
    );
}

// ── the save-scenario harness (save_common::SAME_SCENARIO_CUSTOMER) ──────────

#[test]
fn the_save_scenarios_use_it_exactly_five_times_with_their_causes() {
    let uses: Vec<(&str, usize, usize, &str)> = save_common::SAME_SCENARIO_CUSTOMER
        .iter()
        .map(|s| (s.scenario, s.step, s.same_as, s.reason))
        .collect();
    let demo = "owner 2026-10-09: DEMO shares LIVE";
    let k1 = "K1 (customer per company)";
    assert_eq!(
        uses,
        vec![
            (
                "numbers follow one another through a day, live and demo apart",
                4,
                3,
                demo
            ),
            (
                "the same submission live and demo is two exchanges",
                4,
                3,
                demo
            ),
            (
                "the number is counted for the whole shop, not per branch",
                7,
                3,
                k1
            ),
            (
                "the same submission in two branches is two exchanges",
                7,
                3,
                k1
            ),
            ("a rate of another branch does not count", 7, 3, k1),
        ]
    );
    assert_eq!(
        SAME_CUSTOMER_AS.len() + save_common::SAME_SCENARIO_CUSTOMER.len(),
        8,
        "the cap the lane controller set: eight uses in all"
    );
}

#[test]
#[should_panic(expected = "not the earlier step's customer")]
fn a_scenario_reserve_answering_another_customer_goes_red() {
    save_common::hold_same_scenario_customer("CX-00000002", true, "CX-00000001", "self-test");
}

#[test]
#[should_panic(expected = "already existed")]
fn a_scenario_reserve_answering_existing_false_goes_red() {
    save_common::hold_same_scenario_customer("CX-00000001", false, "CX-00000001", "self-test");
}

#[test]
#[should_panic(expected = "the receipt URL is not the hash of the stored receipt")]
fn a_scenario_receipt_url_that_is_not_the_hash_goes_red() {
    save_common::hold_receipt_hash(
        "/files/receipt-0000",
        b"<html>CX-00000001</html>",
        "self-test",
    );
}

fn customer(id: &str, name: &str, doc: Option<&str>, mode: &str) -> save_common::FxCustomer {
    save_common::FxCustomer {
        id: id.to_owned(),
        name: name.to_owned(),
        birth_date: None,
        passport_expiry: None,
        last_document_ref: doc.map(str::to_owned),
        mode: mode.to_owned(),
    }
}

fn written(customer: &str, doc: &str, name: &str) -> save_common::WrittenSave {
    save_common::WrittenSave {
        customer: customer.to_owned(),
        document_ref: doc.to_owned(),
        name: name.to_owned(),
        birth_date: None,
        passport_expiry: None,
    }
}

#[test]
fn the_fold_takes_the_newest_save_and_drops_the_aliased_entry() {
    let after = vec![
        customer("CX-00000001", "Somchai", Some("XC-1"), "LIVE"),
        customer("TEST-CX-00000001", "Somchai", Some("TEST-XC-1"), "DEMO"),
    ];
    let aliases = std::collections::BTreeMap::from([(
        "TEST-CX-00000001".to_owned(),
        "CX-00000001".to_owned(),
    )]);
    let saves = vec![
        written("CX-00000001", "XC-1", "Somchai"),
        written("CX-00000001", "TEST-XC-1", "Somchai J."),
    ];
    assert_eq!(
        save_common::fold_aliased_customers(&after, &aliases, &saves),
        vec![customer(
            "CX-00000001",
            "Somchai J.",
            Some("TEST-XC-1"),
            "LIVE"
        )]
    );
}

#[test]
fn a_fold_that_takes_a_non_newest_save_goes_red() {
    let after = vec![
        customer("CX-00000001", "Somchai", Some("XC-1"), "LIVE"),
        customer("TEST-CX-00000001", "Somchai", Some("TEST-XC-1"), "DEMO"),
    ];
    let aliases = std::collections::BTreeMap::from([(
        "TEST-CX-00000001".to_owned(),
        "CX-00000001".to_owned(),
    )]);
    let saves = vec![
        written("CX-00000001", "XC-1", "Somchai"),
        written("CX-00000001", "TEST-XC-1", "Somchai J."),
    ];
    // the older save's view of the customer is not what the fold expects
    assert_ne!(
        save_common::fold_aliased_customers(&after, &aliases, &saves),
        vec![customer("CX-00000001", "Somchai", Some("XC-1"), "LIVE")]
    );
}

// ── CompanyCustomers (compat_common::COMPANY_CUSTOMERS_FIXTURES) ────────────

#[test]
fn company_customers_applies_to_exactly_two_fixtures_and_these_steps() {
    assert_eq!(
        compat_common::COMPANY_CUSTOMERS_FIXTURES,
        ["rpc/engine_actions.json", "snapshot/branches.json"]
    );
    assert_eq!(
        compat_common::COMPANY_CUSTOMERS_REASON,
        "list_live company-wide, owner 2026-10-09, (c)"
    );
    let engine = compat_common::Fixture::load("rpc/engine_actions.json");
    assert_eq!(
        compat_common::company_customers_steps(&engine).len(),
        21,
        "engine_actions: the non-MAIN answers carrying customers"
    );
    let branches = compat_common::Fixture::load("snapshot/branches.json");
    assert_eq!(
        compat_common::company_customers_steps(&branches),
        vec![2, 4, 5, 6, 7, 8, 13, 15, 21, 23, 25, 27, 29, 33, 38]
    );
    assert_eq!(
        compat_common::COMPANY_CUSTOMERS_EXCEPTIONS,
        [(
            "snapshot/branches.json",
            31,
            "restricted staff view: empty in v2 and v3"
        )]
    );
    assert_eq!(compat_common::company_customers(&branches, 31), None);
    let other = compat_common::Fixture::load("snapshot/empty.json");
    assert!(compat_common::company_customers_steps(&other).is_empty());
}

#[test]
fn company_customers_takes_the_latest_earlier_recorded_main_list() {
    let branches = compat_common::Fixture::load("snapshot/branches.json");
    assert_eq!(
        compat_common::company_customers(&branches, 13),
        branches.step(11).json().get("customers").cloned(),
        "#13 at B2 takes MAIN's #11"
    );
    assert_eq!(
        compat_common::company_customers(&branches, 2),
        Some(json!([])),
        "before any MAIN customer: the recorded empty list"
    );
    assert_eq!(
        compat_common::company_customers(&branches, 11),
        None,
        "never a MAIN answer"
    );
}

#[test]
#[should_panic(expected = "self-test")]
fn company_customers_with_a_wrong_list_goes_red() {
    let expected = compat_common::with_customers_of(
        &json!({"customers": []}),
        &json!({"customers": [{"id": "CX-00000001"}]}),
        "self-test",
    );
    assert_same(
        &expected,
        &json!({"customers": [{"id": "CX-00000009"}]}),
        "self-test",
    );
}

#[test]
#[should_panic(expected = "self-test")]
fn company_customers_with_a_reordered_list_goes_red() {
    let expected = compat_common::with_customers_of(
        &json!({"customers": []}),
        &json!({"customers": [{"id": "CX-00000001"}, {"id": "CX-00000002"}]}),
        "self-test",
    );
    assert_same(
        &expected,
        &json!({"customers": [{"id": "CX-00000002"}, {"id": "CX-00000001"}]}),
        "self-test",
    );
}

#[test]
#[should_panic(expected = "self-test")]
fn a_main_answer_altered_by_one_customer_goes_red() {
    // the rule never touches a MAIN answer: it is held to its recording as is
    let branches = compat_common::Fixture::load("snapshot/branches.json");
    assert_eq!(compat_common::company_customers(&branches, 11), None);
    let recorded = branches.step(11).json().clone();
    let mut altered = recorded.clone();
    altered["customers"]
        .as_array_mut()
        .expect("a list")
        .push(json!({"id": "CX-00000099"}));
    assert_same(&recorded, &altered, "self-test");
}

#[test]
fn company_customers_rule_covers_every_non_main_customers_answer() {
    for name in compat_common::COMPANY_CUSTOMERS_FIXTURES {
        let fixture = compat_common::Fixture::load(name);
        let found = compat_common::non_main_customers_answers(&fixture);
        let listed = compat_common::company_customers_steps(&fixture);
        let excepted: Vec<usize> = compat_common::COMPANY_CUSTOMERS_EXCEPTIONS
            .iter()
            .filter(|(f, _, _)| *f == name)
            .map(|(_, n, _)| *n)
            .collect();
        for n in &found {
            let in_list = listed.contains(n);
            let in_exceptions = excepted.contains(n);
            assert!(
                in_list != in_exceptions,
                "{name}#{n}: a non-MAIN answer carrying customers must be in exactly one of the list \
                 (here: {in_list}) and the exceptions (here: {in_exceptions})"
            );
        }
        for n in listed.iter().chain(&excepted) {
            assert!(
                found.contains(n),
                "{name}#{n} is listed but is not a non-MAIN answer carrying customers"
            );
        }
    }
    // the lists name only these two fixtures
    for (name, _) in compat_common::COMPANY_CUSTOMERS_STEPS {
        assert!(compat_common::COMPANY_CUSTOMERS_FIXTURES.contains(&name));
    }
    for (name, _, _) in compat_common::COMPANY_CUSTOMERS_EXCEPTIONS {
        assert!(compat_common::COMPANY_CUSTOMERS_FIXTURES.contains(&name));
    }
}

// ── the customer shape of a CompanyCustomers expectation ─────────────────────

#[test]
fn a_read_answer_expects_member_false_and_no_verification_when_the_source_was_a_write() {
    assert_eq!(
        compat_common::MEMBER_SHAPE_REASON,
        "new branch: no member rows; v2 write answers carry no member fields (snapshot.rs write_answer)"
    );
    let source = json!([{"id": "CX-00000001", "name": "Somchai"}]);
    assert_eq!(
        compat_common::customers_shaped_for("adminSnapshot", &source),
        json!([{"id": "CX-00000001", "name": "Somchai", "member": false, "memberVerifiedAt": null}])
    );
    let engine = compat_common::Fixture::load("rpc/engine_actions.json");
    let at_164 = compat_common::company_customers(&engine, 164).expect("#164 is listed");
    assert!(
        at_164
            .as_array()
            .is_some_and(|c| c.len() == 3 && c.iter().all(|c| c["member"] == json!(false))),
        "{at_164}"
    );
}

#[test]
fn a_write_answer_expects_no_member_fields() {
    let source = json!([{"id": "CX-00000001", "member": false, "memberVerifiedAt": null}]);
    assert_eq!(
        compat_common::customers_shaped_for("adminCreateBranch", &source),
        json!([{"id": "CX-00000001"}])
    );
}

#[test]
#[should_panic(expected = "self-test")]
fn a_read_answering_member_true_goes_red() {
    let expected = json!({"customers": compat_common::customers_shaped_for(
        "adminSnapshot",
        &json!([{"id": "CX-00000001"}])
    )});
    assert_same(
        &expected,
        &json!({"customers": [{"id": "CX-00000001", "member": true, "memberVerifiedAt": null}]}),
        "self-test",
    );
}

#[test]
#[should_panic(expected = "self-test")]
fn a_read_missing_member_verified_at_goes_red() {
    let expected = json!({"customers": compat_common::customers_shaped_for(
        "adminSnapshot",
        &json!([{"id": "CX-00000001"}])
    )});
    assert_same(
        &expected,
        &json!({"customers": [{"id": "CX-00000001", "member": false}]}),
        "self-test",
    );
}

#[test]
#[should_panic(expected = "self-test")]
fn a_write_answering_member_goes_red() {
    let expected = json!({"customers": compat_common::customers_shaped_for(
        "adminCreateBranch",
        &json!([{"id": "CX-00000001", "member": false}])
    )});
    assert_same(
        &expected,
        &json!({"customers": [{"id": "CX-00000001", "member": false}]}),
        "self-test",
    );
}

#[test]
fn a_read_copied_from_a_recorded_read_keeps_its_member_values() {
    let source = json!([{"id": "CX-00000001", "member": true, "memberVerifiedAt": "2026-10-01T00:00:00.000Z"}]);
    assert_eq!(
        compat_common::customers_shaped_for("adminSnapshot", &source),
        source
    );
}

#[test]
fn the_member_shape_is_rewritten_on_exactly_engine_164() {
    let engine = compat_common::Fixture::load("rpc/engine_actions.json");
    assert_eq!(
        compat_common::member_shape_rewritten_steps(&engine),
        vec![164]
    );
    let branches = compat_common::Fixture::load("snapshot/branches.json");
    assert_eq!(
        compat_common::member_shape_rewritten_steps(&branches),
        Vec::<usize>::new()
    );
}

#[test]
fn the_write_actions_are_the_adapters_six() {
    // the names `write_answer`'s documentation gives (the adapter keeps no constant)
    let source = include_str!("../src/application/compat/snapshot.rs");
    let doc_end = source
        .find("pub fn write_answer(")
        .expect("snapshot.rs has write_answer");
    let doc_start = source[..doc_end]
        .rfind("/// A snapshot as the six write actions answer it")
        .expect("write_answer's documentation names the six actions");
    let doc = &source[doc_start..doc_end];
    let mut named: Vec<&str> = doc
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|w| w.starts_with("admin"))
        .collect();
    named.sort_unstable();
    let mut ours = compat_common::WRITE_ANSWER_ACTIONS.to_vec();
    ours.sort_unstable();
    assert_eq!(named, ours);
    // and what v2 recorded: a write answer's customers never carry `member`, a read answer's always do
    for name in [
        "rpc/engine_actions.json",
        "snapshot/branches.json",
        "snapshot/transactions.json",
        "snapshot/shift.json",
    ] {
        let fixture = compat_common::Fixture::load(name);
        for step in fixture.steps() {
            let Some(customers) = step
                .body
                .as_ref()
                .and_then(|b| b.get("customers"))
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            let action = step.action.as_deref().unwrap_or("");
            let write = compat_common::WRITE_ANSWER_ACTIONS.contains(&action);
            for c in customers {
                assert_eq!(
                    c.get("member").is_some(),
                    !write,
                    "{name}#{} ({action}): member fields on a {} answer",
                    step.n,
                    if write { "write" } else { "read" }
                );
            }
        }
    }
}
