//! The policy-rows seam (PL-0b #1014, `docs/design/blueprint/broker-seam.md` §11.1).
//!
//! Each area adds its permission rows in its own file here, `areas/<area>.rs`, as
//! `pub const ROWS: &[AreaRow] = &[…];`. `xc-auth/build.rs` generates `AREA_ROWS` from these
//! files, and `Policy::need` consults it before its own table (`Policy::need_table`).

use crate::domain::policy::entities::policy::{Need, Policy};
use crate::domain::policy::value_objects::action::Action;
use crate::domain::policy::value_objects::entry_path::EntryPath;

/// One permission row of an area: what `(path, action)` needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AreaRow {
    pub path: EntryPath,
    pub action: &'static str,
    pub need: Need,
}

include!(concat!(env!("OUT_DIR"), "/area_rows.rs"));

/// Every `(path, action)` that appears in more than one row across `areas`, each reported once,
/// in first-seen order.
pub fn duplicates(areas: &[&[AreaRow]]) -> Vec<(EntryPath, &'static str)> {
    let mut seen = std::collections::HashSet::new();
    let mut order = Vec::new();
    let mut counts = std::collections::HashMap::new();
    for area in areas {
        for row in *area {
            let key = (row.path, row.action);
            *counts.entry(key).or_insert(0) += 1;
            if seen.insert(key) {
                order.push(key);
            }
        }
    }
    order.into_iter().filter(|k| counts[k] > 1).collect()
}

/// Every area row whose `(path, action)` `Policy::need_table` already answers: an area file may
/// only add a row, never override one. In row order.
pub fn shadowed(areas: &[&[AreaRow]]) -> Vec<(EntryPath, &'static str)> {
    let mut shadows = Vec::new();
    for area in areas {
        for row in *area {
            let action = Action::new(row.action);
            if Policy::need_table(row.path, &action).is_some() {
                shadows.push((row.path, row.action));
            }
        }
    }
    shadows
}
