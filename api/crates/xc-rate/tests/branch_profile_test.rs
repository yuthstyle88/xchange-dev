//! OR-5a (#945) first batch — `BranchService::save_profile`, the service under `branchSave`
//! (design `docs/design/blueprint/org-roles-branches.md` §6, OR-1's profile columns).
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own. The routes, views and policy rows are the second
//! batch, written after #767.
//!
//! The save runs in the branch's `BranchTx`, so `branch_versions` moves: that is what the public
//! branch list's ETag (#995) and every branch view's `since` read.
//!
//! Needs `TEST_DATABASE_URL`; nothing skips.

mod common;

use common::*;
use pos_error::AppError;
use serde_json::{Value, json};
use xc_rate::application::branch::service::BranchProfile;

fn profile(code: &str) -> BranchProfile {
    BranchProfile {
        code: code.to_owned(),
        address: Some("1 Silom Rd, Bangkok".to_owned()),
        timezone: "Asia/Bangkok".to_owned(),
        opening_hours: Some(json!({
            "mon": [{ "open": "09:00", "close": "17:00" }],
            "sat": [{ "open": "10:00", "close": "14:00" }]
        })),
        booking_enabled: true,
    }
}

async fn version(db: &TestDb, branch: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE((SELECT version FROM branch_versions WHERE branch_id = $1), 0)",
    )
    .bind(branch)
    .fetch_one(&db.pool)
    .await
    .expect("read branch_versions")
}

/// The profile columns of `branch`, as stored.
async fn stored(db: &TestDb, branch: &str) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('code', code, 'address', address, 'timezone', timezone, \
             'opening_hours', opening_hours, 'booking_enabled', booking_enabled) \
         FROM branches WHERE id = $1",
    )
    .bind(branch)
    .fetch_one(&db.pool)
    .await
    .expect("read the branch")
}

/// A second branch of MAIN's organisation, made the way `create` makes one.
async fn second(db: &TestDb, name: &str) -> String {
    db.branches()
        .create(
            CreateBranch {
                request_id: request_id(),
                name: name.to_owned(),
                from_branch: branch(MAIN),
            },
            ACTOR,
            at(0),
        )
        .await
        .expect("create a branch")
        .id
        .as_str()
        .to_owned()
}

#[tokio::test]
async fn a_saved_profile_is_stored_and_read_back_and_moves_the_branch_version() {
    let db = TestDb::start().await;
    let before = version(&db, MAIN).await;

    let saved = db
        .branches()
        .save_profile(&branch(MAIN), profile("SILOM"))
        .await
        .expect("saved");
    assert_eq!(saved.code.as_deref(), Some("SILOM"));
    assert_eq!(saved.address.as_deref(), Some("1 Silom Rd, Bangkok"));
    assert_eq!(saved.timezone, "Asia/Bangkok");
    assert!(saved.booking_enabled);
    assert_eq!(saved.name, "XC-B", "the name is not part of the profile");

    assert_eq!(
        stored(&db, MAIN).await,
        json!({
            "code": "SILOM",
            "address": "1 Silom Rd, Bangkok",
            "timezone": "Asia/Bangkok",
            "opening_hours": {
                "mon": [{ "open": "09:00", "close": "17:00" }],
                "sat": [{ "open": "10:00", "close": "14:00" }]
            },
            "booking_enabled": true
        })
    );
    let read = db.branches().require(&branch(MAIN)).await.expect("require");
    assert_eq!(read, saved, "require answers what was saved");
    assert_eq!(
        version(&db, MAIN).await,
        before + 1,
        "saved in the branch's BranchTx: its version moves once"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_profile_can_be_cleared_back_to_no_address_and_no_hours() {
    let db = TestDb::start().await;
    db.branches()
        .save_profile(&branch(MAIN), profile("SILOM"))
        .await
        .expect("first save");
    let cleared = BranchProfile {
        code: "SILOM".to_owned(),
        address: None,
        timezone: "Asia/Tokyo".to_owned(),
        opening_hours: None,
        booking_enabled: false,
    };
    db.branches()
        .save_profile(&branch(MAIN), cleared)
        .await
        .expect("saving its own code again is not a conflict");
    assert_eq!(
        stored(&db, MAIN).await,
        json!({
            "code": "SILOM",
            "address": null,
            "timezone": "Asia/Tokyo",
            "opening_hours": null,
            "booking_enabled": false
        })
    );
    db.finish().await;
}

#[tokio::test]
async fn a_bad_zone_bad_hours_or_an_empty_code_is_refused_and_nothing_is_written() {
    let db = TestDb::start().await;
    let before_row = stored(&db, MAIN).await;
    let before_version = version(&db, MAIN).await;

    let mut bad_zone = profile("SILOM");
    bad_zone.timezone = "Mars/Olympus".to_owned();
    let mut bad_hours = profile("SILOM");
    bad_hours.opening_hours = Some(json!({ "mon": [{ "open": "25:00", "close": "17:00" }] }));
    let mut not_an_object = profile("SILOM");
    not_an_object.opening_hours = Some(json!(["09:00-17:00"]));
    let mut no_code = profile("SILOM");
    no_code.code = "   ".to_owned();

    for (what, p) in [
        ("an unknown IANA zone", bad_zone),
        ("hours out of range", bad_hours),
        ("hours that are not an object", not_an_object),
        ("an empty code", no_code),
    ] {
        let out = db.branches().save_profile(&branch(MAIN), p).await;
        assert!(
            matches!(out, Err(AppError::Validation(_))),
            "{what}: expected Validation, got {out:?}"
        );
    }
    assert_eq!(stored(&db, MAIN).await, before_row, "nothing written");
    assert_eq!(version(&db, MAIN).await, before_version, "no version moved");
    db.finish().await;
}

#[tokio::test]
async fn a_code_another_branch_of_the_organisation_holds_is_a_conflict() {
    let db = TestDb::start().await;
    let silom = second(&db, "Silom").await;
    db.branches()
        .save_profile(&branch(&silom), profile("SILOM"))
        .await
        .expect("Silom takes SILOM");
    let before = stored(&db, MAIN).await;

    let out = db
        .branches()
        .save_profile(&branch(MAIN), profile("SILOM"))
        .await;
    assert!(
        matches!(out, Err(AppError::Conflict(_))),
        "expected Conflict, got {out:?}"
    );
    assert_eq!(stored(&db, MAIN).await, before, "nothing written");
    db.finish().await;
}

#[tokio::test]
async fn the_same_code_in_another_organisation_is_not_a_conflict() {
    let db = TestDb::start().await;
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('00000000-0000-0000-0000-0000000000b2', 'YY', NULL, 'Asia/Bangkok', now())",
    )
    .execute(&db.pool)
    .await
    .expect("a second organisation");
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id, code) \
         VALUES ('Y-1', 'Other', '00000000-0000-0000-0000-0000000000b2', 'SILOM')",
    )
    .execute(&db.pool)
    .await
    .expect("its branch, coded SILOM");

    db.branches()
        .save_profile(&branch(MAIN), profile("SILOM"))
        .await
        .expect("codes are unique per organisation only");
    db.finish().await;
}

#[tokio::test]
async fn an_unknown_branch_is_no_such_branch() {
    let db = TestDb::start().await;
    let out = db
        .branches()
        .save_profile(&branch("B-nope"), profile("NOPE"))
        .await;
    match out {
        Err(AppError::Domain(message)) => assert_eq!(message, NO_SUCH_BRANCH),
        other => panic!("expected NO_SUCH_BRANCH, got {other:?}"),
    }
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM branch_versions WHERE branch_id = 'B-nope'")
            .fetch_one(&db.pool)
            .await
            .expect("count branch_versions");
    assert_eq!(
        rows, 0,
        "refused before any BranchTx: no version row for a branch that does not exist"
    );
    db.finish().await;
}
