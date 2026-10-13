//! APP-6 (#997) — the permission rows of `xc-auth/src/domain/policy/areas/assistant.rs` (PL-0b
//! seam, `broker-seam.md` §11.1; design `customer-app.md` §4.4 "Policy rows"; ticket #997 §3).
//! Written before the implementation by the test writer (gemini/warut-1, on the BE
//! lane controller claude/ad05bf8d's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The v3 seam names a command's or view's row by `INHERITS = (EntryPath::Rpc, <name>)`
//! (`api/src/presentation/http/v3/command.rs`, `view.rs`), so the rows are on `EntryPath::Rpc`.
//!
//! Ruling of this test: the assistant command and the conversation view are `Need::Public`.
//! `Need` (`policy.rs`) has no "CUSTOMER or an anonymous app session" variant, and
//! `Need::Permission(CustomerSelf)` would refuse the anonymous session with `signIn`.
//! The owner check is `AssistantService`'s, with the `DraftOwner` the route builds from the
//! actor's `customer_id` or from the `xc_app_sid` cookie (design §3, §4.3).

use xc_auth::domain::policy::areas::{AREA_ROWS, assistant};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

fn need(action: &str) -> Option<Need> {
    Policy::need(EntryPath::Rpc, &Action::new(action))
}

#[test]
fn assistant_ask_is_open_to_a_customer_or_an_app_session() {
    assert_eq!(need("assistantAsk"), Some(Need::Public));
}

#[test]
fn my_assistant_conversation_view_is_open_to_a_customer_or_an_app_session() {
    assert_eq!(need("my_assistant_conversation"), Some(Need::Public));
}

#[test]
fn the_two_rows_live_in_the_assistant_area_file_and_nowhere_else() {
    let mut actions: Vec<&str> = assistant::ROWS.iter().map(|r| r.action).collect();
    actions.sort();
    assert_eq!(actions, vec!["assistantAsk", "my_assistant_conversation"]);
    assert!(assistant::ROWS.iter().all(|r| r.path == EntryPath::Rpc));
    let everywhere = AREA_ROWS
        .iter()
        .flat_map(|area| area.iter())
        .filter(|row| row.path == EntryPath::Rpc && actions.contains(&row.action))
        .count();
    assert_eq!(
        everywhere, 2,
        "AREA_ROWS carries the two rows once, from assistant.rs only"
    );
}
