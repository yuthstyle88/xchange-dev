//! Tests for ticket CS-2: `PgShiftTemplateStore` (docs/design/configurable-shifts.md §2.8, §2.9, §3.2, §4 CS-2).

mod shift_service_common;

use chrono::{NaiveTime, Utc};
use shift_service_common::TestDb;
use xc_kernel::ids::BranchId;
use xc_shift::domain::shift_plan::repository::ShiftTemplateStore;
use xc_shift::domain::shift_plan::shift_plan::ShiftPlan;
use xc_shift::domain::shift_plan::shift_template::ShiftTemplateInput;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).expect("valid time")
}

#[tokio::test]
async fn s1_seed_defaults_in_on_a_branch_with_no_live_row() {
    let db = TestDb::start().await;
    let store = PgShiftTemplateStore;
    let branch = db.branch("S1").await;

    // The migration trigger seeds A, B, C on branch insert; retire them to test a branch with no live rows.
    sqlx::query("UPDATE shift_templates SET retired_at = now() WHERE branch_id = $1")
        .bind(branch.as_str())
        .execute(&db.pool)
        .await
        .unwrap();

    let mut tx = db.pool.begin().await.unwrap();
    let seeded = store.seed_defaults_in(&mut tx, &branch, ORG).await.unwrap();
    assert!(seeded, "first call on empty branch returns true");
    tx.commit().await.unwrap();

    let mut tx = db.pool.begin().await.unwrap();
    let live = store.live_in(&mut tx, &branch).await.unwrap();
    let defaults = ShiftPlan::defaults();
    assert_eq!(live.len(), 3);
    for (i, (actual, expected)) in live.iter().zip(defaults.iter()).enumerate() {
        assert_eq!(actual.name, expected.name);
        assert_eq!(actual.starts_at, expected.starts_at);
        assert_eq!(actual.ends_at, expected.ends_at);
        assert_eq!(actual.break_window, expected.break_window);
        assert_eq!(actual.position, i as i16);
    }

    let seeded_again = store.seed_defaults_in(&mut tx, &branch, ORG).await.unwrap();
    assert!(!seeded_again, "second call returns false");
    drop(tx);

    db.drop_schema().await;
}

#[tokio::test]
async fn s2_replace_in_with_updated_and_new() {
    let db = TestDb::start().await;
    let store = PgShiftTemplateStore;
    let branch = db.branch("S2").await;

    let mut tx = db.pool.begin().await.unwrap();
    let initial = store.live_in(&mut tx, &branch).await.unwrap();
    assert_eq!(initial.len(), 3);
    let id_a = initial[0].id.clone();
    let id_b = initial[1].id.clone();
    let id_c = initial[2].id.clone();

    let inputs = vec![
        ShiftTemplateInput {
            id: Some(id_a.clone()),
            name: "A-Updated".to_owned(),
            starts_at: hm(7, 0),
            ends_at: hm(15, 0),
            break_window: None,
        },
        ShiftTemplateInput {
            id: None,
            name: "D".to_owned(),
            starts_at: hm(15, 0),
            ends_at: hm(23, 0),
            break_window: None,
        },
    ];

    let now = Utc::now();
    let live = store
        .replace_in(&mut tx, &branch, ORG, &inputs, now)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(live.len(), 2);
    assert_eq!(live[0].id, id_a);
    assert_eq!(live[0].name, "A-Updated");
    assert_eq!(live[0].position, 0);

    assert_eq!(live[1].name, "D");
    assert_eq!(live[1].position, 1);

    // Verify B and C got retired
    let retired_count: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM shift_templates WHERE id = ANY($1) AND retired_at IS NOT NULL",
    )
    .bind(vec![
        uuid::Uuid::parse_str(&id_b).unwrap(),
        uuid::Uuid::parse_str(&id_c).unwrap(),
    ])
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(retired_count.0, 2);

    db.drop_schema().await;
}

#[tokio::test]
async fn s2b_replace_in_swapping_names() {
    let db = TestDb::start().await;
    let store = PgShiftTemplateStore;
    let branch = db.branch("S2b").await;

    let mut tx = db.pool.begin().await.unwrap();
    let initial = store.live_in(&mut tx, &branch).await.unwrap();
    let id_a = initial[0].id.clone();
    let id_b = initial[1].id.clone();
    let id_c = initial[2].id.clone();

    // Swapping names: kept row A gets name "B", kept row B gets name "A", C is kept with "C"
    let inputs = vec![
        ShiftTemplateInput {
            id: Some(id_a.clone()),
            name: "B".to_owned(),
            starts_at: hm(14, 0),
            ends_at: hm(22, 0),
            break_window: None,
        },
        ShiftTemplateInput {
            id: Some(id_b.clone()),
            name: "A".to_owned(),
            starts_at: hm(6, 0),
            ends_at: hm(14, 0),
            break_window: None,
        },
        ShiftTemplateInput {
            id: Some(id_c.clone()),
            name: "C".to_owned(),
            starts_at: hm(22, 0),
            ends_at: hm(6, 0),
            break_window: None,
        },
    ];

    let live = store
        .replace_in(&mut tx, &branch, ORG, &inputs, Utc::now())
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(live.len(), 3);
    assert_eq!(live[0].id, id_a);
    assert_eq!(live[0].name, "B");
    assert_eq!(live[0].position, 0);

    assert_eq!(live[1].id, id_b);
    assert_eq!(live[1].name, "A");
    assert_eq!(live[1].position, 1);

    db.drop_schema().await;
}

#[tokio::test]
async fn s3_replace_in_with_id_of_another_branch() {
    let db = TestDb::start().await;
    let store = PgShiftTemplateStore;
    let branch1 = db.branch("S3_1").await;
    let branch2 = db.branch("S3_2").await;

    let mut tx = db.pool.begin().await.unwrap();
    let live2 = store.live_in(&mut tx, &branch2).await.unwrap();
    let id_from_branch2 = live2[0].id.clone();

    let inputs = vec![ShiftTemplateInput {
        id: Some(id_from_branch2),
        name: "Steal".to_owned(),
        starts_at: hm(6, 0),
        ends_at: hm(14, 0),
        break_window: None,
    }];

    let res = store
        .replace_in(&mut tx, &branch1, ORG, &inputs, Utc::now())
        .await;
    assert!(
        matches!(res, Err(pos_error::AppError::Validation(ref msg)) if msg == "shiftTemplateUnknown"),
        "expected shiftTemplateUnknown, got: {res:?}"
    );
    // Caller rolls back
    drop(tx);

    // Verify nothing changed on branch 1
    let mut tx = db.pool.begin().await.unwrap();
    let live1 = store.live_in(&mut tx, &branch1).await.unwrap();
    assert_eq!(live1.len(), 3);
    assert_eq!(live1[0].name, "A");
    drop(tx);

    db.drop_schema().await;
}

#[tokio::test]
async fn s4_main_after_migrating() {
    let db = TestDb::start().await;
    let store = PgShiftTemplateStore;
    let mut tx = db.pool.begin().await.unwrap();
    let live = store
        .live_in(&mut tx, &BranchId::new("MAIN"))
        .await
        .unwrap();
    drop(tx);

    let defaults = ShiftPlan::defaults();
    assert_eq!(live.len(), defaults.len());
    for (actual, expected) in live.iter().zip(defaults.iter()) {
        assert_eq!(actual.name, expected.name);
        assert_eq!(actual.starts_at, expected.starts_at);
        assert_eq!(actual.ends_at, expected.ends_at);
        assert_eq!(actual.break_window, expected.break_window);
    }
    db.drop_schema().await;
}

#[tokio::test]
async fn s5_a_break_stored() {
    let db = TestDb::start().await;
    let store = PgShiftTemplateStore;
    let branch = db.branch("S5").await;

    // Retire existing seeded templates first
    sqlx::query("UPDATE shift_templates SET retired_at = now() WHERE branch_id = $1")
        .bind(branch.as_str())
        .execute(&db.pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO shift_templates (organization_id, branch_id, name, starts_at, ends_at, break_starts_at, break_ends_at, position) \
         VALUES ('00000000-0000-0000-0000-000000000001'::uuid, $1, 'Morning', '08:00', '16:00', '12:00', '13:00', 0)",
    )
    .bind(branch.as_str())
    .execute(&db.pool)
    .await
    .unwrap();

    let mut tx = db.pool.begin().await.unwrap();
    let live = store.live_in(&mut tx, &branch).await.unwrap();
    drop(tx);

    assert_eq!(live.len(), 1);
    assert_eq!(live[0].name, "Morning");
    assert_eq!(live[0].break_window, Some((hm(12, 0), hm(13, 0))));
    db.drop_schema().await;
}
