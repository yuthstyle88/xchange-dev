//! Retired for RE-7 #604 by the ticket author (`docs/design/blueprint/rates.md` §9, "Provided
//! tests that change": `api/tests/v3_admin_rates_view_test.rs` | the direct save | RE-7 / RE-5 |
//! publications).
//!
//! This file pinned ticket #468's `admin_rates` and `admin_rate_history` views and the direct
//! save (`/api/rpc` `adminSaveRate`) that wrote what they read. RE-7 drops `rates` (§8 migration
//! 6), removes `adminSaveRate` and deletes both views with their routes; the web reads neither
//! since RE-5. Every test here was checked for something that still exists:
//! - who may read the two views, their tables, their history pages and their generated types:
//!   the views are deleted;
//! - ticket §8's saves as `RATES` / the owner: the save is removed (pinned gone by
//!   `rates_re7_retire_test.rs`);
//! - §4.2 "every writer moves the version": measured through `admin_rates`; a publication moving
//!   the branch version is pinned by `v3_kiosk_currencies_publication_test.rs` and
//!   `v3_admin_bookings_view_test.rs`, the kiosk `configure` switch by
//!   `v3_kiosk_currencies_view_test.rs`, `adminCreateBranch` by `v3_admin_branches_view_test.rs`.
//!
//! The rates the admin screens show now come from publications (`published_rates`,
//! `rate_publications`, RE-3 / RE-5), whose tests are those files' own. No test is left here.
