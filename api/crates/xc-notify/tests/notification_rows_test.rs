//! APP-3 (#993): the notification policy rows in their own area file,
//! `xc-auth/src/domain/policy/areas/notifications.rs` (design `customer-app.md` §4.4,
//! `broker-seam.md` §11.1; 89bd61d9's re-point on #993, 2026-10-10T08:04Z). Each is `SignedIn`;
//! "own rows only" is `NotificationService`'s (`read_test.rs`). Written before the implementation
//! by the test author (claude/dell-claude-2). Do not edit, delete or weaken.
//!
//! The path is `EntryPath::Rpc`, the path every v3 view and command inherits today
//! (`consent_texts.rs:33`, `approval_decide.rs:52`) and the one `need_table` leaves open for new
//! actions (`policy.rs`: `EntryPath::Rpc => match act { … _ => None }`). xc-auth's own
//! `area_rows_test.rs` checks the rows for duplicates and shadowing.

use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::policy::areas::notifications::ROWS;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

const ACTIONS: [&str; 3] = ["notificationRead", "notificationReadAll", "myNotifications"];

#[test]
fn each_notification_action_needs_a_signed_in_user() {
    for action in ACTIONS {
        assert_eq!(
            Policy::need(EntryPath::Rpc, &Action::new(action)),
            Some(Need::Permission(Permission::SignedIn)),
            "{action}"
        );
    }
}

#[test]
fn the_area_file_holds_exactly_the_three_rows() {
    let mut actions: Vec<&str> = ROWS.iter().map(|r| r.action).collect();
    actions.sort_unstable();
    let mut expected = ACTIONS.to_vec();
    expected.sort_unstable();
    assert_eq!(actions, expected);
    assert!(ROWS.iter().all(|r| r.path == EntryPath::Rpc));
}
