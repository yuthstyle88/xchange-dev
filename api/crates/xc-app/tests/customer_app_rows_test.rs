//! APP-4 (#994) — the permission rows of `xc-auth/src/domain/policy/areas/customer_app.rs` (PL-0b
//! seam, `broker-seam.md` §11.1; design `customer-app.md` §4.4 "Policy rows"; ticket #994 §3).
//! Written before the implementation by the ticket author's delegate (claude/dell-claude-3). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! The v3 seam names a command's or view's row by `INHERITS = (EntryPath::Rpc, <name>)`
//! (`api/src/presentation/http/v3/command.rs`, `view.rs`), so the rows are on `EntryPath::Rpc`.
//!
//! Ruling of this test (commented on #994 for the lane controller): the draft commands, the compute
//! and the view are `Need::Public`. `Need` (`policy.rs`) has no "CUSTOMER or an anonymous app
//! session" variant, and `Need::Permission(CustomerSelf)` would refuse the anonymous session with
//! `signIn`. The owner check is `PlannerService`'s, with the `DraftOwner` the route builds from the
//! actor's `customer_id` or from the `xc_app_sid` cookie (design §3, §4.3). `appSettingsSave` is
//! `Permission::SystemSettings` (ADMIN).

use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::policy::areas::{AREA_ROWS, customer_app};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

fn need(action: &str) -> Option<Need> {
    Policy::need(EntryPath::Rpc, &Action::new(action))
}

#[test]
fn planner_draft_save_is_open_to_a_customer_or_an_app_session() {
    assert_eq!(need("plannerDraftSave"), Some(Need::Public));
}

#[test]
fn planner_draft_delete_is_open_to_a_customer_or_an_app_session() {
    assert_eq!(need("plannerDraftDelete"), Some(Need::Public));
}

#[test]
fn the_planner_preview_compute_is_open_to_anyone() {
    assert_eq!(need("planner_preview"), Some(Need::Public));
}

#[test]
fn the_my_planner_drafts_view_is_open_to_a_customer_or_an_app_session() {
    assert_eq!(need("my_planner_drafts"), Some(Need::Public));
}

#[test]
fn app_settings_save_needs_system_settings() {
    assert_eq!(
        need("appSettingsSave"),
        Some(Need::Permission(Permission::SystemSettings))
    );
}

#[test]
fn the_five_rows_live_in_the_customer_app_area_file_and_nowhere_else() {
    let mut actions: Vec<&str> = customer_app::ROWS.iter().map(|r| r.action).collect();
    actions.sort();
    assert_eq!(
        actions,
        vec![
            "appSettingsSave",
            "my_planner_drafts",
            "plannerDraftDelete",
            "plannerDraftSave",
            "planner_preview"
        ]
    );
    assert!(customer_app::ROWS.iter().all(|r| r.path == EntryPath::Rpc));
    let everywhere = AREA_ROWS
        .iter()
        .flat_map(|area| area.iter())
        .filter(|row| row.path == EntryPath::Rpc && actions.contains(&row.action))
        .count();
    assert_eq!(
        everywhere, 5,
        "AREA_ROWS carries the five rows once, from customer_app.rs only"
    );
}
