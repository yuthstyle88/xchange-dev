//! Up-front tests for PT-14 (#3968, `docs/design/blueprint/pricing-tiers.md` §PT-14, permission row line 210):
//! the `MarketRatesRefresh` permission, `RATES_STOCK` at `CREATE` (`pricing.market:refresh`).
//! Written before the implementation by the test writer claude/studio-claude-1 for the lane controller claude/a90ab16c.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `Permission::MarketRatesRefresh` does not exist before the fix, so this file does not compile until it does.
//!
//! What they pin (the ticket's §2):
//! - `Permission::ALL` holds it exactly once, right after the other rates permissions (after
//!   `OrgStartingPricesManage`), and grows 51 → 52;
//! - `PermissionModule::module` maps it to `(RatesStock, Create)`;
//! - `RoleGrants::seed` gives it to every built-in group whose `RATES_STOCK` level is `CREATE` or `FULL`
//!   (`role_grants.rs` `seed`: EXECUTIVE and RATE_MANAGER, both `FULL`) and to no other (FINANCE and
//!   VERIFICATION_REVIEWER are at `VIEW`; ADMIN, BRANCH_MANAGER and TELLER have no `RATES_STOCK`; MARKETING and
//!   CUSTOMER keep their hand-written sets). Nobody adds it by hand: it follows the module level.
//! - No role's module levels move: it only adds a permission inside a level the holders already have.

use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::permission_module::{
    Level, Module, PermissionMatrix, PermissionModule,
};
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::RoleGrants;

use Permission as P;

/// Every built-in group and whether its seed holds `MarketRatesRefresh`, with its `RATES_STOCK` level in
/// `RoleGrants::seed` (`None` = no `RATES_STOCK`, or a hand-written set).
const HOLDERS: [(Role, Option<Level>, bool); 9] = [
    (Role::Executive, Some(Level::Full), true),
    (Role::Admin, None, false),
    (Role::RateManager, Some(Level::Full), true),
    (Role::BranchManager, None, false),
    (Role::Teller, None, false),
    (Role::Finance, Some(Level::View), false),
    (Role::Marketing, None, false),
    (Role::Customer, None, false),
    (Role::VerificationReviewer, Some(Level::View), false),
];

#[test]
fn pt14_permission_all_holds_market_rates_refresh_exactly_once() {
    let n = Permission::ALL
        .iter()
        .filter(|p| **p == P::MarketRatesRefresh)
        .count();
    assert_eq!(n, 1, "MarketRatesRefresh appears once in Permission::ALL");
    // RBAC §11.9 (AP-0) appends the nine approval kinds after it: 52 -> 61.
    assert_eq!(
        Permission::ALL.len(),
        61,
        "Permission::ALL grows 51 -> 52 (PT-14), then 52 -> 61 (§11.9)"
    );
}

#[test]
fn pt14_market_rates_refresh_comes_right_after_the_other_rates_permissions() {
    let at = |p: Permission| {
        Permission::ALL
            .iter()
            .position(|q| *q == p)
            .unwrap_or_else(|| panic!("{p:?} in Permission::ALL"))
    };
    assert_eq!(
        at(P::MarketRatesRefresh),
        at(P::OrgStartingPricesManage) + 1,
        "MarketRatesRefresh follows OrgStartingPricesManage, the last rates permission"
    );
}

#[test]
fn pt14_market_rates_refresh_is_rates_stock_at_create() {
    assert_eq!(
        PermissionModule::module(P::MarketRatesRefresh),
        Some((Module::RatesStock, Level::Create))
    );
}

#[test]
fn pt14_the_seed_gives_it_to_rates_stock_create_or_full_and_to_nobody_else() {
    for (role, rates_stock, holds) in HOLDERS {
        assert_eq!(
            RoleGrants::seed(role).contains(&P::MarketRatesRefresh),
            holds,
            "{} (RATES_STOCK {:?}) holds MarketRatesRefresh: {holds}",
            role.as_str(),
            rates_stock
        );
    }
    let listed: Vec<Role> = HOLDERS.iter().map(|(r, _, _)| *r).collect();
    assert_eq!(
        listed,
        RoleGrants::BUILT_IN.to_vec(),
        "the table covers every built-in group"
    );
}

/// The seed derives from module levels (`role_grants.rs` `seed`), so the holders of `MarketRatesRefresh` are
/// exactly the holders of the other `RATES_STOCK` `CREATE` permission, `RatesDraft`.
#[test]
fn pt14_its_holders_are_the_holders_of_rates_draft() {
    for role in RoleGrants::BUILT_IN {
        let seed = RoleGrants::seed(role);
        assert_eq!(
            seed.contains(&P::MarketRatesRefresh),
            seed.contains(&P::RatesDraft),
            "{}: MarketRatesRefresh follows RATES_STOCK CREATE like RatesDraft",
            role.as_str()
        );
    }
}

/// Adding the permission moves no role's `RATES_STOCK` level: the matrix row is still what the seed's modules say.
#[test]
fn pt14_no_rates_stock_level_moves() {
    for (role, rates_stock, _) in HOLDERS {
        if !PermissionMatrix::ROLES.contains(&role) {
            continue;
        }
        assert_eq!(
            PermissionMatrix::of(role).get(&Module::RatesStock).copied(),
            rates_stock,
            "{} RATES_STOCK level",
            role.as_str()
        );
    }
}
