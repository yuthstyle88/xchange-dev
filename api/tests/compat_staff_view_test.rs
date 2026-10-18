//! T11s — the staff-path view of the v2 compatibility layer.
//!
//! v2 answers `/api/staff/rpc` with what `/api/rpc` would answer, changed in two
//! ways (v2 `server.mjs`, the staff bridge): `adminSnapshot` gains the caller and
//! the branch's employees and loses what a RATES or SYSTEM employee may not see,
//! and every answer has its links turned to the staff paths. These tests hold
//! `staff_view` and `rewrite_urls` to every recorded pair of answers. No database.

mod compat_common;

use std::collections::{BTreeMap, BTreeSet};

use compat_common::{Fixture, Person, RPC_FILES, SNAPSHOT_FILES, Step};
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::application::compat::staff_view::{StaffMember, rewrite_urls, staff_view};

const RPC: &str = "/api/rpc";
const STAFF_RPC: &str = "/api/staff/rpc";

fn step_files() -> Vec<&'static str> {
    SNAPSHOT_FILES
        .iter()
        .chain(RPC_FILES.iter())
        .copied()
        .collect()
}

fn v3_role(role: &str) -> Option<xc_auth::domain::access::Role> {
    use xc_auth::domain::access::Role as R;
    match role {
        "FRONT" => Some(R::Teller),
        "BACKEND" | "MANAGER" => Some(R::BranchManager),
        "RATES" => Some(R::RateManager),
        "FINANCE" => Some(R::Finance),
        "SYSTEM" => Some(R::Admin),
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => Some(R::VerificationReviewer),
        _ => None,
    }
}

fn member(person: &Person) -> StaffMember {
    StaffMember {
        id: person.id.clone(),
        username: person.username.clone(),
        name: person.name.clone(),
        role: v3_role(&person.role)
            .unwrap_or_else(|| panic!("{}: unknown role {}", person.key, person.role)),
        role_text: person.role.clone(),
        branch: BranchId::new(
            person
                .branch_id
                .clone()
                .unwrap_or_else(|| panic!("{}: an employee without a branch", person.key)),
        ),
    }
}

/// Every employee of the file, of every branch, in the order they were created.
fn everybody(fixture: &Fixture) -> Vec<StaffMember> {
    fixture
        .people
        .iter()
        .filter(|p| !p.is_owner())
        .map(member)
        .collect()
}

/// One state read by one employee on both routes.
struct Pair<'a> {
    file: &'a str,
    me: StaffMember,
    rpc: &'a Step,
    staff: &'a Step,
}

impl Pair<'_> {
    fn name(&self) -> String {
        format!(
            "{}#{} → #{} ({})",
            self.file, self.rpc.n, self.staff.n, self.staff.sent_as
        )
    }
}

/// The pairs of accepted reads of `action` with the same `view` and `as`: the
/// answer of `/api/staff/rpc` with the last answer of `/api/rpc` before it.
fn pairs<'a>(fixture: &'a Fixture, action: &str) -> Vec<Pair<'a>> {
    let mut last: BTreeMap<(&str, &str), &Step> = BTreeMap::new();
    let mut found = Vec::new();
    for step in fixture.steps() {
        let (Some(view), Some(200)) = (step.view.as_deref(), step.status) else {
            continue;
        };
        if step.action.as_deref() != Some(action) {
            continue;
        }
        let key = (view, step.sent_as.as_str());
        match step.path() {
            Some(RPC) => {
                last.insert(key, step);
            }
            Some(STAFF_RPC) => {
                if let Some(rpc) = last.get(&key) {
                    let person = fixture
                        .sender(step)
                        .unwrap_or_else(|| panic!("{}#{}: nobody sent it", fixture.name, step.n));
                    found.push(Pair {
                        file: &fixture.name,
                        me: member(person),
                        rpc,
                        staff: step,
                    });
                }
            }
            _ => {}
        }
    }
    found
}

fn fixtures() -> Vec<Fixture> {
    step_files().into_iter().map(Fixture::load).collect()
}

fn viewed(fixture: &Fixture, pair: &Pair) -> Value {
    staff_view(pair.rpc.json().clone(), &pair.me, &everybody(fixture))
}

fn length(value: &Value) -> usize {
    value
        .as_array()
        .unwrap_or_else(|| panic!("not an array: {value}"))
        .len()
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {value}"))
        .keys()
        .map(String::as_str)
        .collect()
}

fn a_member(id: &str, role: Role, branch: &str) -> StaffMember {
    StaffMember {
        id: id.to_string(),
        username: id.to_lowercase(),
        name: format!("{id} name"),
        role: v3_role(role.as_str()).expect("staff role"),
        role_text: role.as_str().to_string(),
        branch: BranchId::new(branch),
    }
}

/// A small snapshot with something in every part a role can lose.
fn a_snapshot() -> Value {
    json!({
        "version": 1,
        "branchId": "MAIN",
        "actor": "e1",
        "transactions": [{"id": "XC-1", "slipUrl": "/files/slip-1"}, {"id": "XC-2", "slipUrl": null}],
        "customers": [{"id": "C1"}],
        "rates": [{"code": "USD", "buy": 32.1}],
        "rateHistory": [{"code": "USD"}],
        "stock": {"shifts": [{"id": "SH-1"}], "movements": [{"id": "MV-1"}]},
        "sheetUrl": "/#transactions",
        "deskUrl": "/desk?branch=MAIN",
        "operations": {
            "enabled": true,
            "phase": "OPEN",
            "safe": {"THB": 1},
            "target": {"amounts": {"THB": 2}, "by": "E2", "at": "t"},
            "entity": {"company": "c", "site": "s"},
            "custodian": "E2",
            "attendance": [
                {"id": "A1", "employeeId": "E1", "out": null},
                {"id": "A2", "employeeId": "E2", "out": null},
                {"id": "A3", "employeeId": "E1", "out": "t"}
            ],
            "requests": [{"id": "R1", "evidenceKeys": ["/files/e1"]}],
            "events": [{"id": "EV1"}],
            "cutoff": {"at": "t"},
            "exceptions": [{"id": "X1"}],
            "holds": [{"id": "H1"}],
            "drafts": [{"id": "D1"}],
            "serviceOpenedAt": "t"
        }
    })
}

// ── the recorded pairs ───────────────────────────────────────────────────────

#[test]
fn every_recorded_staff_snapshot_is_the_rpc_snapshot_viewed() {
    let mut count = 0;
    let mut per_file = BTreeMap::new();
    let mut roles = BTreeSet::new();
    for fixture in fixtures() {
        for pair in pairs(&fixture, "adminSnapshot") {
            let actual = viewed(&fixture, &pair);
            assert!(
                actual == *pair.staff.json(),
                "{}: staff_view of the /api/rpc answer is not what v2 answered on /api/staff/rpc",
                pair.name()
            );
            count += 1;
            *per_file.entry(fixture.name.clone()).or_insert(0) += 1;
            roles.insert(pair.me.role_text.clone());
        }
    }
    let expected_per_file: BTreeMap<String, i32> = [
        ("rpc/ops_actions.json", 1),
        ("snapshot/branches.json", 4),
        ("snapshot/empty.json", 9),
        ("snapshot/rates.json", 3),
        ("snapshot/shift.json", 11),
        ("snapshot/transactions.json", 6),
    ]
    .into_iter()
    .map(|(name, n)| (name.to_string(), n))
    .collect();
    assert_eq!(per_file, expected_per_file);
    assert_eq!(count, 34, "the generator checked its claim on 34 pairs");
    let nine: BTreeSet<String> = Role::STAFF.iter().map(|r| r.as_str().to_string()).collect();
    assert_eq!(
        roles, nine,
        "every role an employee can hold is among the pairs"
    );
}

#[test]
fn every_recorded_staff_desk_config_is_the_rpc_one_with_its_urls_rewritten() {
    let mut count = 0;
    for fixture in fixtures() {
        for pair in pairs(&fixture, "adminDeskConfig") {
            assert_eq!(pair.me.role_text, "FRONT", "{}", pair.name());
            assert_eq!(pair.staff.json()["adminUrl"], json!("/staff"));
            assert_ne!(pair.rpc.json()["adminUrl"], json!("/staff"));
            assert!(
                rewrite_urls(pair.rpc.json().clone()) == *pair.staff.json(),
                "{}: rewrite_urls of the /api/rpc desk config is not the staff one",
                pair.name()
            );
            count += 1;
        }
    }
    assert_eq!(
        count, 10,
        "9 in the snapshot files, 1 in rpc/ops_actions.json"
    );
}

/// `value` with every staff file link turned back into the link `/api/rpc` gives.
fn as_rpc_links(value: &Value, turned: &mut usize) -> Value {
    match value {
        Value::String(text) => match text.strip_prefix("/api/staff/file/") {
            Some(rest) => {
                *turned += 1;
                Value::String(format!("/files/{rest}"))
            }
            None => value.clone(),
        },
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| as_rpc_links(v, turned)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), as_rpc_links(v, turned)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[test]
fn the_documents_answers_have_their_links_rewritten_inside_nested_arrays() {
    let mut answers = 0;
    let mut links = 0;
    for fixture in fixtures() {
        for step in fixture.steps() {
            if step.action.as_deref() != Some("documents") || step.status != Some(200) {
                continue;
            }
            assert_eq!(step.path(), Some(STAFF_RPC), "{}#{}", fixture.name, step.n);
            let recorded = step.json();
            let rows = recorded["links"]
                .as_array()
                .unwrap_or_else(|| panic!("{}#{}: no links", fixture.name, step.n));
            assert!(rows.iter().all(|row| {
                row[1]
                    .as_str()
                    .is_some_and(|url| url.starts_with("/api/staff/file/"))
            }));
            let mut turned = 0;
            let before = as_rpc_links(recorded, &mut turned);
            assert_eq!(turned, rows.len());
            assert_ne!(before, *recorded);
            assert_eq!(
                rewrite_urls(before),
                *recorded,
                "{}#{}",
                fixture.name,
                step.n
            );
            answers += 1;
            links += turned;
        }
    }
    // snapshot/transactions.json#51 and #52, rpc/engine_actions.json#127
    assert_eq!(answers, 3);
    assert_eq!(links, 7);
}

#[test]
fn a_save_answer_of_the_rpc_route_gets_the_staff_links() {
    let mut seen = 0;
    for fixture in fixtures() {
        for step in fixture.steps() {
            if step.action.as_deref() != Some("saveTransaction")
                || step.status != Some(200)
                || step.path() != Some(RPC)
            {
                continue;
            }
            let before = step.json();
            let after = rewrite_urls(before.clone());
            for (key, from, to) in [
                ("passportUrl", "/files/", "/api/staff/file/"),
                ("receiptUrl", "/files/", "/api/staff/file/"),
                ("slipUrl", "/files/", "/api/staff/file/"),
                ("folderUrl", "/documents?", "/staff/documents?"),
            ] {
                let Some(old) = before.get(key).and_then(Value::as_str) else {
                    continue;
                };
                let expected = match old.strip_prefix(from) {
                    Some(rest) => format!("{to}{rest}"),
                    // `/documents` alone (a DEMO save) is not a link the bridge turns
                    None => old.to_string(),
                };
                assert_eq!(
                    after[key],
                    json!(expected),
                    "{}#{} {key}",
                    fixture.name,
                    step.n
                );
                seen += 1;
            }
            assert_ne!(before["sheetUrl"], json!("/staff"));
            assert_eq!(after["sheetUrl"], json!("/staff"));
            // nothing else moved
            let mut expected = before.clone();
            for key in [
                "passportUrl",
                "receiptUrl",
                "slipUrl",
                "folderUrl",
                "sheetUrl",
            ] {
                if before.get(key).is_some() {
                    expected[key] = after[key].clone();
                }
            }
            assert_eq!(after, expected, "{}#{}", fixture.name, step.n);
        }
    }
    assert!(seen >= 14, "only {seen} links seen");
}

#[test]
fn a_recorded_staff_answer_is_left_as_it_is() {
    let mut count = 0;
    for fixture in fixtures() {
        for step in fixture.steps() {
            if step.path() != Some(STAFF_RPC) || step.status != Some(200) {
                continue;
            }
            let Some(body) = step.body.as_ref() else {
                continue;
            };
            assert!(
                rewrite_urls(body.clone()) == *body,
                "{}#{}: an answer v2 already rewrote changes when rewritten again",
                fixture.name,
                step.n
            );
            count += 1;
        }
    }
    assert!(count >= 110, "only {count} staff answers");
}

// ── the other seven roles ────────────────────────────────────────────────────

#[test]
fn the_other_seven_roles_lose_nothing() {
    let snapshot = a_snapshot();
    for role in Role::STAFF {
        if matches!(role, Role::Rates | Role::System) {
            continue;
        }
        let me = a_member("E1", role, "MAIN");
        let view = staff_view(snapshot.clone(), &me, std::slice::from_ref(&me));
        let mut expected = rewrite_urls(snapshot.clone());
        expected["employee"] = json!({
            "id": "E1", "username": "e1", "name": "E1 name", "role": role.as_str(),
            "branchId": "MAIN", "active": true, "mustChange": false
        });
        expected["employees"] = json!([{"id": "E1", "name": "E1 name", "role": role.as_str()}]);
        assert_eq!(view, expected, "{role:?}");
        assert_eq!(length(&view["transactions"]), 2, "{role:?}");
        assert_eq!(length(&view["operations"]["attendance"]), 3, "{role:?}");
        assert_eq!(view["operations"]["enabled"], json!(true), "{role:?}");
    }
}

#[test]
fn rates_on_the_small_snapshot() {
    let me = a_member("E1", Role::Rates, "MAIN");
    let view = staff_view(a_snapshot(), &me, std::slice::from_ref(&me));
    let truth = a_snapshot();
    assert_eq!(view["transactions"], json!([]));
    assert_eq!(view["customers"], json!([]));
    assert_eq!(view["rates"], truth["rates"]);
    assert_eq!(view["rateHistory"], truth["rateHistory"]);
    assert_eq!(view["stock"], truth["stock"]);
    assert_eq!(
        view["operations"],
        json!({
            "enabled": true,
            "phase": "OPEN",
            "safe": {"THB": 1},
            "target": {"amounts": {"THB": 2}, "by": "E2", "at": "t"},
            "entity": {"company": "c", "site": "s"},
            "custodian": "E2",
            "attendance": [
                {"id": "A1", "employeeId": "E1", "out": null},
                {"id": "A3", "employeeId": "E1", "out": "t"}
            ],
            "requests": [],
            "events": [],
            "cutoff": null,
            "exceptions": [],
            "holds": [],
            "drafts": [{"id": "D1"}],
            "serviceOpenedAt": "t"
        })
    );
}

#[test]
fn system_on_the_small_snapshot() {
    let me = a_member("E1", Role::System, "MAIN");
    let view = staff_view(a_snapshot(), &me, std::slice::from_ref(&me));
    assert_eq!(
        view,
        json!({
            "version": 1,
            "branchId": "MAIN",
            "actor": "e1",
            "transactions": [],
            "customers": [],
            "rates": [],
            "rateHistory": [],
            "stock": {"shifts": [], "movements": []},
            "sheetUrl": "/staff",
            "deskUrl": "/staff/desk",
            "operations": {
                "enabled": false,
                "phase": "CLOSED",
                "entity": {"company": "c", "site": "s"}
            },
            "employee": {
                "id": "E1", "username": "e1", "name": "E1 name", "role": "SYSTEM",
                "branchId": "MAIN", "active": true, "mustChange": false
            },
            "employees": [{"id": "E1", "name": "E1 name", "role": "SYSTEM"}]
        })
    );
}

#[test]
fn system_is_told_enabled_false_when_custody_is_off_too_and_a_null_entity_stays_null() {
    let me = a_member("E1", Role::System, "MAIN");
    let mut snapshot = a_snapshot();
    snapshot["operations"]["enabled"] = json!(false);
    snapshot["operations"]["entity"] = Value::Null;
    let view = staff_view(snapshot, &me, &[]);
    assert_eq!(
        view["operations"],
        json!({"enabled": false, "phase": "CLOSED", "entity": null})
    );
}

// ── the caller and the colleagues ────────────────────────────────────────────

#[test]
fn the_caller_is_added_with_seven_keys() {
    let fixture = Fixture::load("snapshot/empty.json");
    let pair = pairs(&fixture, "adminSnapshot")
        .into_iter()
        .next()
        .expect("a pair in snapshot/empty.json");
    assert!(pair.rpc.json().get("employee").is_none());
    assert!(pair.rpc.json().get("employees").is_none());
    let view = viewed(&fixture, &pair);
    assert_eq!(
        view["employee"],
        json!({
            "id": pair.me.id,
            "username": pair.me.username,
            "name": pair.me.name,
            "role": pair.me.role_text.as_str(),
            "branchId": "MAIN",
            "active": true,
            "mustChange": false
        })
    );
    assert_eq!(keys(&view["employee"]).len(), 7);
    assert_eq!(view["employee"], pair.staff.json()["employee"]);
}

#[test]
fn the_colleagues_are_the_employees_of_the_callers_branch_in_the_order_given() {
    let fixture = Fixture::load("snapshot/branches.json");
    let everybody = everybody(&fixture);
    let branches: BTreeSet<&str> = everybody.iter().map(|m| m.branch.as_str()).collect();
    assert_eq!(branches.len(), 2, "employees of two branches: {branches:?}");
    let pairs = pairs(&fixture, "adminSnapshot");
    assert_eq!(pairs.len(), 4);
    let mut lists = BTreeSet::new();
    for pair in &pairs {
        let view = viewed(&fixture, pair);
        let expected: Vec<Value> = everybody
            .iter()
            .filter(|m| m.branch == pair.me.branch)
            .map(|m| json!({"id": m.id, "name": m.name, "role": m.role_text.as_str()}))
            .collect();
        assert!(expected.len() < everybody.len());
        assert_eq!(view["employees"], Value::Array(expected), "{}", pair.name());
        assert_eq!(view["employees"], pair.staff.json()["employees"]);
        lists.insert(view["employees"].to_string());
    }
    assert_eq!(lists.len(), 2, "one list per branch");
}

#[test]
fn colleagues_keep_the_order_they_are_given_in_and_have_three_keys() {
    let me = a_member("E1", Role::Front, "MAIN");
    let colleagues = [
        a_member("Z9", Role::DocumentReview, "MAIN"),
        a_member("E7", Role::Backend, "B-2"),
        me.clone(),
        a_member("A0", Role::Manager, "MAIN"),
    ];
    let view = staff_view(json!({}), &me, &colleagues);
    assert_eq!(
        view["employees"],
        json!([
            {"id": "Z9", "name": "Z9 name", "role": "DOCUMENT_REVIEW"},
            {"id": "E1", "name": "E1 name", "role": "FRONT"},
            {"id": "A0", "name": "A0 name", "role": "MANAGER"}
        ])
    );
}

#[test]
fn what_the_snapshot_says_of_employees_is_replaced() {
    let me = a_member("E1", Role::Front, "MAIN");
    let view = staff_view(
        json!({"employee": {"id": "somebody else"}, "employees": [1, 2, 3]}),
        &me,
        &[],
    );
    assert_eq!(view["employee"]["id"], json!("E1"));
    assert_eq!(view["employees"], json!([]));
}

#[test]
fn the_snapshot_links_are_the_staff_ones() {
    let me = a_member("E1", Role::Front, "MAIN");
    let view = staff_view(a_snapshot(), &me, &[]);
    assert_eq!(view["sheetUrl"], json!("/staff"));
    assert_eq!(view["deskUrl"], json!("/staff/desk"));
    assert_eq!(
        view["transactions"][0]["slipUrl"],
        json!("/api/staff/file/slip-1")
    );
    assert_eq!(view["transactions"][1]["slipUrl"], Value::Null);
    assert_eq!(
        view["operations"]["requests"][0]["evidenceKeys"],
        json!(["/api/staff/file/e1"])
    );
}

#[test]
fn a_name_that_looks_like_a_link_is_rewritten_as_v2_rewrites_the_whole_answer() {
    let mut me = a_member("E1", Role::Front, "MAIN");
    me.name = "/files/odd".to_string();
    let view = staff_view(json!({}), &me, std::slice::from_ref(&me));
    assert_eq!(view["employee"]["name"], json!("/api/staff/file/odd"));
    assert_eq!(view["employees"][0]["name"], json!("/api/staff/file/odd"));
}

// ── a snapshot that is not what the builder makes ────────────────────────────

#[test]
fn rates_without_operations_gets_none_and_does_not_panic() {
    let me = a_member("E1", Role::Rates, "MAIN");
    let view = staff_view(json!({"transactions": [1], "customers": [2]}), &me, &[]);
    assert_eq!(view["transactions"], json!([]));
    assert_eq!(view["customers"], json!([]));
    assert!(view.get("operations").is_none());
}

#[test]
fn rates_with_operations_that_lack_a_list_still_hides_all_six() {
    let me = a_member("E1", Role::Rates, "MAIN");
    let view = staff_view(
        json!({"operations": {"enabled": true, "attendance": "not a list"}}),
        &me,
        &[],
    );
    assert_eq!(
        view["operations"],
        json!({
            "enabled": true, "attendance": [], "requests": [], "events": [],
            "exceptions": [], "holds": [], "cutoff": null
        })
    );
    let view = staff_view(
        json!({"operations": {"attendance": [{"id": "A"}, 7, {"employeeId": "E1"}, {"employeeId": 1}]}}),
        &me,
        &[],
    );
    assert_eq!(
        view["operations"]["attendance"],
        json!([{"employeeId": "E1"}])
    );
}

#[test]
fn rates_with_operations_that_is_no_object_is_shown_none_of_it() {
    let me = a_member("E1", Role::Rates, "MAIN");
    let view = staff_view(json!({"operations": [{"requests": ["secret"]}]}), &me, &[]);
    assert_eq!(view["operations"], Value::Null);
}

#[test]
fn system_without_operations_gets_the_three_keys_with_a_null_entity() {
    let me = a_member("E1", Role::System, "MAIN");
    for snapshot in [
        json!({}),
        json!({"operations": null}),
        json!({"operations": "x"}),
    ] {
        let view = staff_view(snapshot, &me, &[]);
        assert_eq!(
            view["operations"],
            json!({"enabled": false, "phase": "CLOSED", "entity": null})
        );
        assert_eq!(view["stock"], json!({"shifts": [], "movements": []}));
        assert_eq!(view["rates"], json!([]));
    }
}

#[test]
fn a_snapshot_that_is_no_object_has_only_its_urls_rewritten() {
    let me = a_member("E1", Role::System, "MAIN");
    assert_eq!(
        staff_view(json!(["/files/a", 1]), &me, &[]),
        json!(["/api/staff/file/a", 1])
    );
    assert_eq!(staff_view(Value::Null, &me, &[]), Value::Null);
}

// ── rewrite_urls, case by case ───────────────────────────────────────────────

#[test]
fn a_string_is_rewritten_only_when_it_begins_with_the_path() {
    for (before, after) in [
        ("/files/abc", "/api/staff/file/abc"),
        ("/files/", "/api/staff/file/"),
        ("/files/a?download=1", "/api/staff/file/a?download=1"),
        // only the beginning: v2 replaces the first occurrence, which is the prefix
        ("/files/a/files/b", "/api/staff/file/a/files/b"),
        ("see /files/abc", "see /files/abc"),
        (" /files/abc", " /files/abc"),
        ("https://desk.test/files/abc", "https://desk.test/files/abc"),
        ("/files", "/files"),
        ("/filesystem/abc", "/filesystem/abc"),
        ("/Files/abc", "/Files/abc"),
        ("files/abc", "files/abc"),
        (
            "/documents?branch=MAIN&ref=XC-1",
            "/staff/documents?branch=MAIN&ref=XC-1",
        ),
        ("/documents?", "/staff/documents?"),
        (
            "/documents?a=/documents?b",
            "/staff/documents?a=/documents?b",
        ),
        ("/documents", "/documents"),
        ("/documents/x?y", "/documents/x?y"),
        ("see /documents?ref=1", "see /documents?ref=1"),
        ("/api/staff/file/abc", "/api/staff/file/abc"),
        ("/staff/documents?ref=1", "/staff/documents?ref=1"),
        ("", ""),
        ("พาสปอร์ต /files/x", "พาสปอร์ต /files/x"),
    ] {
        assert_eq!(rewrite_urls(json!(before)), json!(after), "{before:?}");
        assert_eq!(
            rewrite_urls(json!([[before]])),
            json!([[after]]),
            "{before:?}"
        );
        assert_eq!(
            rewrite_urls(json!({"k": before})),
            json!({"k": after}),
            "{before:?}"
        );
    }
}

#[test]
fn the_three_named_keys_are_replaced_whatever_they_hold_at_any_depth() {
    assert_eq!(
        rewrite_urls(json!({
            "adminUrl": "/?branch=MAIN",
            "sheetUrl": "/#transactions",
            "deskUrl": "/desk?branch=MAIN",
            "inside": [{"adminUrl": null, "deeper": {"deskUrl": 5, "sheetUrl": {"a": "/files/x"}}}],
        })),
        json!({
            "adminUrl": "/staff",
            "sheetUrl": "/staff",
            "deskUrl": "/staff/desk",
            "inside": [{"adminUrl": "/staff", "deeper": {"deskUrl": "/staff/desk", "sheetUrl": "/staff"}}],
        })
    );
    // the names are exact
    assert_eq!(
        rewrite_urls(json!({"adminurl": "a", "AdminUrl": "b", "deskUrls": "c", "xsheetUrl": "d"})),
        json!({"adminurl": "a", "AdminUrl": "b", "deskUrls": "c", "xsheetUrl": "d"})
    );
}

#[test]
fn keys_and_what_is_not_text_are_left_alone() {
    let value = json!({
        "/files/key": 1,
        "/documents?key": true,
        "n": 32.1,
        "i": -5,
        "big": 3209999999.68,
        "nothing": null,
        "no": false,
        "empty": [],
        "none": {},
    });
    assert_eq!(rewrite_urls(value.clone()), value);
    for plain in [json!(1), json!(true), Value::Null, json!([]), json!({})] {
        assert_eq!(rewrite_urls(plain.clone()), plain);
    }
}

#[test]
fn links_inside_arrays_inside_arrays_are_rewritten() {
    assert_eq!(
        rewrite_urls(json!({
            "ref": "XC-1",
            "links": [["พาสปอร์ต", "/files/p"], ["ใบเสร็จ", "/files/receipt-1"], ["x", ["/files/deep", {"u": "/documents?r=1"}]]]
        })),
        json!({
            "ref": "XC-1",
            "links": [
                ["พาสปอร์ต", "/api/staff/file/p"],
                ["ใบเสร็จ", "/api/staff/file/receipt-1"],
                ["x", ["/api/staff/file/deep", {"u": "/staff/documents?r=1"}]]
            ]
        })
    );
}
