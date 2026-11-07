//! CA-3a: measured same-origin pages the staff desk and sign-in flow navigate to, and pages that
//! stay refused (`api/src/presentation/pages.rs`, `web/pages/*`).

use url::Url;
use xc_counter::navigation::{NavDecision, decide};

fn check(rows: &[&str], expected: NavDecision) {
    let server = Url::parse("https://xc-v3.spacing.cloud").expect("server");
    for row in rows {
        let url = Url::parse(row).expect(row);
        assert_eq!(decide(&url, &server), expected, "{row}");
    }
}

#[test]
fn measured_desk_and_sign_in_pages_are_allowed() {
    check(
        &[
            // /desk (pages.rs:174, web/pages/desk.html)
            "https://xc-v3.spacing.cloud/desk",
            "https://xc-v3.spacing.cloud/desk?branch=MAIN&kiosk=req-1",
            // /desk/backdated (web/pages/desk-backdated.html, linked from DeskPage.tsx:78)
            "https://xc-v3.spacing.cloud/desk/backdated",
            // /desk/bookings (web/pages/desk-bookings.html)
            "https://xc-v3.spacing.cloud/desk/bookings",
            // /desk/queue (web/pages/desk-queue.html)
            "https://xc-v3.spacing.cloud/desk/queue",
            // /desk/transactions/{id} (web/pages/desk-transaction.html, linked from RefundsStep.tsx:82 and BackdatedScreen.tsx:735)
            "https://xc-v3.spacing.cloud/desk/transactions/00000000-0000-0000-0000-000000000001",
            // /staff/desk (pages.rs:262, web/pages/staff-desk.html)
            "https://xc-v3.spacing.cloud/staff/desk",
            // /kiosk-staff (capabilities/bridge.json:9, navigation_test.rs:8)
            "https://xc-v3.spacing.cloud/kiosk-staff",
            "https://xc-v3.spacing.cloud/kiosk-staff/a",
            // /login (pages.rs:206,789, web/pages/login.html, DeskShiftGate.tsx:32, AccountBar.tsx:93, QueueCard.tsx:236)
            "https://xc-v3.spacing.cloud/login",
            "https://xc-v3.spacing.cloud/login?next=%2Fdesk",
            "https://xc-v3.spacing.cloud/login?next=%2Fstaff%2Fdesk",
        ],
        NavDecision::Allow,
    );
}

#[test]
fn non_desk_and_customer_pages_and_segment_prefixes_are_refused() {
    check(
        &[
            // Admin, staff floor/report/documents/guide, devices, kiosk, API, and customer pages
            "https://xc-v3.spacing.cloud/",
            "https://xc-v3.spacing.cloud/admin",
            "https://xc-v3.spacing.cloud/staff",
            "https://xc-v3.spacing.cloud/staff/report",
            "https://xc-v3.spacing.cloud/staff/documents",
            "https://xc-v3.spacing.cloud/staff-guide",
            "https://xc-v3.spacing.cloud/staff-guide.html",
            "https://xc-v3.spacing.cloud/devices",
            "https://xc-v3.spacing.cloud/documents",
            "https://xc-v3.spacing.cloud/receipt-sample",
            "https://xc-v3.spacing.cloud/operations/verification",
            "https://xc-v3.spacing.cloud/kiosk",
            "https://xc-v3.spacing.cloud/kiosk-queue.html",
            "https://xc-v3.spacing.cloud/kiosk-currencies.html",
            "https://xc-v3.spacing.cloud/app",
            "https://xc-v3.spacing.cloud/app/sign-in",
            "https://xc-v3.spacing.cloud/rates",
            "https://xc-v3.spacing.cloud/queue-display",
            "https://xc-v3.spacing.cloud/q",
            "https://xc-v3.spacing.cloud/c",
            "https://xc-v3.spacing.cloud/api/v3/views/me",
            // Whole-segment boundary checks
            "https://xc-v3.spacing.cloud/desktop",
            "https://xc-v3.spacing.cloud/login-other",
            "https://xc-v3.spacing.cloud/staff/desktop",
            "https://xc-v3.spacing.cloud/kiosk-staffing",
        ],
        NavDecision::Refuse,
    );
}
