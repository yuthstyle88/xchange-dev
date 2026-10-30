//! #5733 (owner ruling A on #5683, 2026-10-30; `docs/design/blueprint/pricing-formula.md` §5 F-2): the
//! organisation formula's save and its `ORGANISATION` preview follow the group's `PricingFormulaSave`, not the role
//! name EXECUTIVE. A group given it saves; an EXECUTIVE group that has had it taken off is refused
//! `roleNotAllowedHere`.
//! Written before the implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL`. `RoleGrants::install` is process-global: every test here holds `LOCK` and puts the
//! seeded snapshot back when it passes. CI runs nextest, one process per test, so other files never see these
//! snapshots; under a plain `cargo test` the files of a binary share one process and may interleave.

mod common;
mod pricing_common;
mod pricing_formula_api;

use pricing_common::{Room, command, owner, rate_manager};
use pricing_formula_api::*;
use serde_json::json;
use xc_auth::domain::access::{Group, Permission, Role, RoleGrants, RoleGrantsSnapshot};

/// Async, because each test holds it across its requests.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Puts the seeded snapshot back when a test passes (installing while unwinding would abort the binary).
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            RoleGrants::install(RoleGrantsSnapshot::seeded());
        }
    }
}

/// The built-in groups at their seed, with `PricingFormulaSave` added to `gains` and taken off `loses`.
fn install(gains: Option<Role>, loses: Option<Role>) {
    let groups: Vec<(Group, Vec<Permission>)> = RoleGrants::BUILT_IN
        .iter()
        .map(|&r| {
            let mut set = RoleGrants::seed(r);
            if Some(r) == loses {
                set.retain(|p| *p != Permission::PricingFormulaSave);
            }
            if Some(r) == gains && !set.contains(&Permission::PricingFormulaSave) {
                set.push(Permission::PricingFormulaSave);
            }
            (Group::built_in(r), set)
        })
        .collect();
    RoleGrants::install(RoleGrantsSnapshot::new(41, groups));
}

/// MAIN (0 / 0), PFB (+0.3 / −0.3), the 10-28 BOT line, as `pricing_formula_save_command_test`.
async fn room() -> Room {
    let room = Room::start().await;
    bot_1028(&room).await;
    policy(&room, "MAIN", "0", "0", wide(), false).await;
    policy(&room, PFB, "0.3000", "-0.3000", wide(), false).await;
    room
}

async fn preview_org(room: &Room, who: &pricing_common::Who) -> pricing_common::Answer {
    command(
        room,
        who,
        PREVIEW,
        "MAIN",
        json!({ "buyFormula": "ref_buy", "sellFormula": "ref_sell", "scope": "ORGANISATION" }),
        None,
    )
    .await
}

/// Row 1: a RATE_MANAGER whose group has been given `PricingFormulaSave` saves the organisation formula
/// (today: `roleNotAllowedHere` from `pricing_formula_save.rs:74-81`).
#[actix_web::test]
async fn a_rate_manager_group_given_pricing_formula_save_saves_the_formula() {
    let _l = LOCK.lock().await;
    let _r = Restore;
    install(Some(Role::RateManager), None);
    let room = room().await;
    let body = save(&room, &rate_manager(), "ref_buy", "ref_sell")
        .await
        .ok("RATE_MANAGER with PricingFormulaSave");
    assert_eq!(body["version"], json!(1), "{body}");
    assert_eq!(room.count("pricing_formula_versions").await, 1);
    room.finish().await;
}

/// Row 2: an EXECUTIVE whose group has had `PricingFormulaSave` taken off is refused `roleNotAllowedHere`, and
/// nothing is written (today: saved, because the role name is EXECUTIVE).
#[actix_web::test]
async fn an_executive_group_without_pricing_formula_save_is_refused() {
    let _l = LOCK.lock().await;
    let _r = Restore;
    install(None, Some(Role::Executive));
    let room = room().await;
    let a = save(&room, &owner(), "ref_buy", "ref_sell").await;
    refused_role(&a, "EXECUTIVE without PricingFormulaSave");
    assert_eq!(room.count("pricing_formula_versions").await, 0);
    room.finish().await;
}

/// Row 3: the `ORGANISATION` preview follows the same permission: a RATE_MANAGER given it gets the table
/// (today: `roleNotAllowedHere` from `pricing_formula_preview.rs:122-128`).
#[actix_web::test]
async fn the_organisation_preview_answers_a_rate_manager_group_given_pricing_formula_save() {
    let _l = LOCK.lock().await;
    let _r = Restore;
    install(Some(Role::RateManager), None);
    let room = room().await;
    let body = preview_org(&room, &rate_manager())
        .await
        .ok("ORGANISATION preview, RATE_MANAGER with PricingFormulaSave");
    assert!(body["rows"].is_array(), "{body}");
    assert_eq!(
        room.count("pricing_formula_versions").await,
        0,
        "a preview writes nothing"
    );
    room.finish().await;
}

/// Row 4: an EXECUTIVE whose group has had it taken off is refused the `ORGANISATION` preview (today: answered).
#[actix_web::test]
async fn the_organisation_preview_refuses_an_executive_group_without_pricing_formula_save() {
    let _l = LOCK.lock().await;
    let _r = Restore;
    install(None, Some(Role::Executive));
    let room = room().await;
    let a = preview_org(&room, &owner()).await;
    refused_role(
        &a,
        "ORGANISATION preview, EXECUTIVE without PricingFormulaSave",
    );
    room.finish().await;
}
