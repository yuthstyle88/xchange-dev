//! XC-BE-GAPS-A row 9 (owner ruling R2, 2026-10-27: "kioskBookingCheckIn becomes staff-only"): the policy row.
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 9, 2026-10-27). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design `docs/design/blueprint/kiosk-shared-phone.md` R-1 (`:23`, no customer kiosk at the branch) and R-8 (`:30`,
//! only staff issue numbers; the kiosk `confirm` needs `QueueOperate`, D19 `:235`). The booking check-in that issues
//! a BOOKING number takes the same permission as `confirm`: `Need::Permission(Permission::QueueOperate)`, not
//! `Need::Public` (today's `areas/bookings.rs:24-28`). The HTTP refusals are pinned in
//! `api/tests/gapsa9_kiosk_booking_check_in_staff_only_test.rs`.
//!
//! The roles named in the HTTP test's header are pinned here from the seed, so the header cannot drift from it:
//! QueueOperate is held by EXECUTIVE, RATE_MANAGER, BRANCH_MANAGER and TELLER; not by ADMIN, FINANCE,
//! VERIFICATION_REVIEWER, MARKETING or CUSTOMER.

use xc_auth::domain::access::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

#[test]
fn gapsa9_kiosk_booking_check_in_needs_queue_operate() {
    assert_eq!(
        Policy::need(EntryPath::Rpc, &Action::new("kioskBookingCheckIn")),
        Some(Need::Permission(Permission::QueueOperate)),
        "R-8: only staff who operate the queue issue a BOOKING number"
    );
    assert_eq!(
        Policy::need(EntryPath::Kiosk, &Action::new("confirm")),
        Some(Need::Permission(Permission::QueueOperate)),
        "the same permission as the kiosk `confirm` (D19)"
    );

    let holders: Vec<Role> = RoleGrants::BUILT_IN
        .into_iter()
        .filter(|r| RoleGrants::seed(*r).contains(&Permission::QueueOperate))
        .collect();
    assert_eq!(
        holders,
        vec![
            Role::Executive,
            Role::RateManager,
            Role::BranchManager,
            Role::Teller
        ],
        "the seeded holders of QueueOperate, in BUILT_IN order"
    );
}
