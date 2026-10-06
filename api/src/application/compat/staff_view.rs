//! The staff-path view of a snapshot: role stripping and URL rewriting.
//!
//! v2 answers `/api/staff/rpc` with what `/api/rpc` answers, passed through its
//! staff bridge. The bridge does two things, and this file is both:
//!
//! - [`staff_view`] — for `adminSnapshot` only: the caller and the branch's
//!   employees are added, and a RATES or SYSTEM employee loses what the role may
//!   not see; then the links are rewritten;
//! - [`rewrite_urls`] — for the answer of every other action: the links only.
//!
//! Both are pure functions over `serde_json::Value`. Who may call which action
//! is `Policy`'s; which employees are active is the caller's.

use serde_json::{Map, Value, json};
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;

/// Where the staff path serves an uploaded file.
const STAFF_FILE: &str = "/api/staff/file/";
/// Where the owner's path serves it.
const OWNER_FILE: &str = "/files/";
/// The staff path's documents page, with its query.
const STAFF_DOCUMENTS: &str = "/staff/documents?";
/// The owner's documents page, with its query.
const OWNER_DOCUMENTS: &str = "/documents?";
/// The staff page: what `adminUrl` and `sheetUrl` become.
const STAFF_HOME: &str = "/staff";
/// The staff desk: what `deskUrl` becomes.
const STAFF_DESK: &str = "/staff/desk";

/// The lists of `operations` a RATES employee is shown empty.
const HIDDEN_LISTS: [&str; 4] = ["requests", "events", "exceptions", "holds"];

/// An employee as the staff path names them.
///
/// Only an active employee whose first password is changed reaches
/// `/api/staff/rpc` (v2 refuses the others before the bridge), so the view
/// always says `active: true` and `mustChange: false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaffMember {
    pub id: String,
    pub username: String,
    pub name: String,
    pub role: Role,
    pub branch: BranchId,
}

/// The answer of `adminSnapshot` on `/api/staff/rpc`, from the answer `/api/rpc`
/// gives the same employee at the same instant.
///
/// `colleagues` are the active employees in the order the list is to show them
/// (v2: the order they were created in); those of another branch than `me`'s
/// are left out. `me` is listed only if it is among them.
///
/// - every role: `employee` (seven keys) and `employees` (`id`, `name`, `role`)
///   are set, replacing what the snapshot may say;
/// - RATES and SYSTEM: `transactions` and `customers` are empty; in `operations`
///   only their own `attendance` stays, `requests`, `events`, `exceptions` and
///   `holds` are empty and `cutoff` is null;
/// - SYSTEM: `stock`, `rates` and `rateHistory` are empty as well, and
///   `operations` is exactly `{enabled: false, phase: "CLOSED", entity}` —
///   `enabled` is false whatever the truth;
/// - then [`rewrite_urls`] over the whole answer.
///
/// A snapshot that is not an object only has its links rewritten.
pub fn staff_view(snapshot: Value, me: &StaffMember, colleagues: &[StaffMember]) -> Value {
    let Value::Object(mut snapshot) = snapshot else {
        return rewrite_urls(snapshot);
    };
    let employees: Vec<Value> = colleagues
        .iter()
        .filter(|colleague| colleague.branch == me.branch)
        .map(|colleague| {
            json!({
                "id": colleague.id,
                "name": colleague.name,
                "role": colleague.role.as_str(),
            })
        })
        .collect();
    snapshot.insert("employees".to_string(), Value::Array(employees));
    snapshot.insert(
        "employee".to_string(),
        json!({
            "id": me.id,
            "username": me.username,
            "name": me.name,
            "role": me.role.as_str(),
            "branchId": me.branch.as_str(),
            "active": true,
            "mustChange": false,
        }),
    );
    if matches!(me.role, Role::Rates | Role::System) {
        snapshot.insert("transactions".to_string(), json!([]));
        snapshot.insert("customers".to_string(), json!([]));
    }
    match me.role {
        Role::Rates => {
            if let Some(operations) = snapshot.get_mut("operations") {
                hide_the_floor(operations, &me.id);
            }
        }
        Role::System => {
            snapshot.insert("stock".to_string(), json!({"shifts": [], "movements": []}));
            snapshot.insert("rates".to_string(), json!([]));
            snapshot.insert("rateHistory".to_string(), json!([]));
            let entity = snapshot
                .get_mut("operations")
                .and_then(|operations| operations.get_mut("entity"))
                .map(Value::take)
                .unwrap_or(Value::Null);
            snapshot.insert(
                "operations".to_string(),
                json!({"enabled": false, "phase": "CLOSED", "entity": entity}),
            );
        }
        _ => {}
    }
    rewrite_urls(Value::Object(snapshot))
}

/// What a RATES employee is shown of `operations`: only their own attendance,
/// no request, event, exception or hold, no cut-off. The other keys stay.
///
/// `operations` that is not an object is shown as null: nothing of it is known
/// to be safe to show.
fn hide_the_floor(operations: &mut Value, employee_id: &str) {
    let Value::Object(map) = operations else {
        *operations = Value::Null;
        return;
    };
    let own: Vec<Value> = match map.remove("attendance") {
        Some(Value::Array(records)) => records
            .into_iter()
            .filter(|record| record.get("employeeId").and_then(Value::as_str) == Some(employee_id))
            .collect(),
        _ => Vec::new(),
    };
    map.insert("attendance".to_string(), Value::Array(own));
    for list in HIDDEN_LISTS {
        map.insert(list.to_string(), json!([]));
    }
    map.insert("cutoff".to_string(), Value::Null);
}

/// The answer of any action on `/api/staff/rpc`, from the answer `/api/rpc`
/// gives: every link is turned to the staff path, at any depth.
///
/// - a string that **begins** with `/files/` begins with `/api/staff/file/`;
/// - a string that begins with `/documents?` begins with `/staff/documents?`
///   (`/documents` alone stays);
/// - in an object, `adminUrl` and `sheetUrl` become `"/staff"` and `deskUrl`
///   becomes `"/staff/desk"`, whatever they held;
/// - keys, numbers, booleans and null are left as they are.
pub fn rewrite_urls(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(rewrite_text(text)),
        Value::Array(items) => Value::Array(items.into_iter().map(rewrite_urls).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let value = match key.as_str() {
                        "adminUrl" | "sheetUrl" => Value::String(STAFF_HOME.to_string()),
                        "deskUrl" => Value::String(STAFF_DESK.to_string()),
                        _ => rewrite_urls(value),
                    };
                    (key, value)
                })
                .collect::<Map<String, Value>>(),
        ),
        other => other,
    }
}

fn rewrite_text(text: String) -> String {
    if let Some(rest) = text.strip_prefix(OWNER_FILE) {
        format!("{STAFF_FILE}{rest}")
    } else if let Some(rest) = text.strip_prefix(OWNER_DOCUMENTS) {
        format!("{STAFF_DOCUMENTS}{rest}")
    } else {
        text
    }
}
