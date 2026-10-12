//! Which filter keys each A01 drill-down target accepts (design §4.4, ruling G2 on #1071), and the drill-down
//! A01 sends: only the keys its target accepts, and the rest listed as `dropped`.

use std::collections::BTreeMap;

/// The filter keys the paged view `view` accepts today: design §4.4's "Passed" column, read from each target's
/// filter DTO on `main` (plus `branch`, the query key every branch view takes). Each EX-11 child (#1103–#1106)
/// replaces its target's arm with that DTO's `DRILLDOWN_KEYS`.
pub fn accepted_keys(view: &str) -> &'static [&'static str] {
    match view {
        // `AdminTransactionsFilter` (`dto/v3/admin_transactions.rs:32`)
        "admin_transactions" => &[
            "branch", "from", "to", "q", "type", "status", "staff", "staffQ", "currency", "channel",
        ],
        // `AdminCustomersFilter` (`dto/v3/admin_customers.rs:32`): organisation-wide, `q` only
        "admin_customers" => &["q"],
        // `ApprovalRequestsFilter` (`dto/v3/approval_requests.rs:10-13`, G3)
        "approval_requests" => &["branch", "status", "type"],
        // CA-8 #624's view is not on `main` yet
        "cash_counts" => &[],
        // `AdminCashFilter` (`dto/v3/admin_cash.rs:12`)
        "admin_cash" => &["branch", "all"],
        // `AdminOverviewFilter` (`dto/v3/admin_overview.rs:15`)
        "admin_overview" => &["branch", "from", "to"],
        _ => &[],
    }
}

/// A drill-down: the paged view, the filter it is sent, and the keys it would not take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drilldown {
    pub view: &'static str,
    pub filter: BTreeMap<&'static str, String>,
    pub dropped: Vec<&'static str>,
}

impl Drilldown {
    /// The drill-down to `view` for the keys A01 would pass, in A01's key order: the ones `view` accepts go in
    /// `filter`, the others in `dropped`, in the same order.
    pub fn to(view: &'static str, keys: Vec<(&'static str, String)>) -> Drilldown {
        let accepted = accepted_keys(view);
        let mut filter = BTreeMap::new();
        let mut dropped = Vec::new();
        for (key, value) in keys {
            if accepted.contains(&key) {
                filter.insert(key, value);
            } else {
                dropped.push(key);
            }
        }
        Drilldown {
            view,
            filter,
            dropped,
        }
    }
}
