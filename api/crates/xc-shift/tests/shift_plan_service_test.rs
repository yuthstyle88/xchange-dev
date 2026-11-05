//! Up-front tests for ticket CS-3: `ShiftPlanService` (docs/design/configurable-shifts.md §2.3, §2.9, §4 rows V1–V6),
//! on the owner's 24-hour rule: every plan covers all 24 hours.
//! Written by the ticket author claude/35271802 — the implementer must not edit, delete or weaken these.
//! They need CS-1 (`ShiftPlan`) and CS-2 (`PgShiftTemplateStore`) merged into `feat/configurable-shifts`.

mod shift_service_common;

use std::sync::Arc;

use chrono::{DateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Asia::Bangkok;
use pos_error::AppError;
use shift_service_common::TestDb;
use xc_kernel::ids::BranchId;
use xc_shift::application::shift_plan::service::ShiftPlanService;
use xc_shift::domain::shift_plan::shift_template::ShiftTemplateInput;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn bkk(d: u32, h: u32, m: u32) -> DateTime<Utc> {
    Bangkok
        .with_ymd_and_hms(2026, 11, d, h, m, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

fn row(name: &str, start: NaiveTime, end: NaiveTime) -> ShiftTemplateInput {
    ShiftTemplateInput {
        id: None,
        name: name.to_owned(),
        starts_at: start,
        ends_at: end,
        break_window: None,
    }
}

/// An overlapping 24-hour plan: A 06–15, B 14–23, C 22–07 (cutover 06:00).
fn overlapping() -> Vec<ShiftTemplateInput> {
    vec![
        row("A", hm(6, 0), hm(15, 0)),
        row("B", hm(14, 0), hm(23, 0)),
        row("C", hm(22, 0), hm(7, 0)),
    ]
}

fn service(db: &TestDb) -> ShiftPlanService {
    ShiftPlanService::new(Arc::new(PgShiftTemplateStore), db.branch_tx())
}

/// A test branch with the overlapping plan saved through the service.
async fn branch_with_plan(db: &TestDb, svc: &ShiftPlanService) -> BranchId {
    let branch = db.branch("CS3").await;
    let mut tx = db.pool.begin().await.unwrap();
    svc.save_in(&mut tx, &branch, ORG, overlapping(), Utc::now())
        .await
        .expect("save the overlapping plan");
    tx.commit().await.unwrap();
    branch
}

async fn live_names(db: &TestDb, branch: &BranchId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT name FROM shift_templates WHERE branch_id = $1 AND retired_at IS NULL ORDER BY position",
    )
    .bind(branch.as_str())
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn v1_an_invalid_list_is_refused_with_its_reason_and_index_and_nothing_is_written() {
    let db = TestDb::start().await;
    let svc = service(&db);
    let branch = branch_with_plan(&db, &svc).await;

    let mut tx = db.pool.begin().await.unwrap();
    let got = svc
        .save_in(
            &mut tx,
            &branch,
            ORG,
            vec![
                row("A", hm(6, 0), hm(18, 0)),
                row("  ", hm(18, 0), hm(6, 0)),
            ],
            Utc::now(),
        )
        .await;
    drop(tx);
    match got {
        Err(AppError::Validation(msg)) => assert_eq!(msg, "shiftPlanInvalid:emptyName:1"),
        other => panic!("expected shiftPlanInvalid, got {other:?}"),
    }

    let mut tx = db.pool.begin().await.unwrap();
    let got = svc.save_in(&mut tx, &branch, ORG, vec![], Utc::now()).await;
    drop(tx);
    match got {
        Err(AppError::Validation(msg)) => assert_eq!(msg, "shiftPlanInvalid:none:"),
        other => panic!("expected shiftPlanInvalid, got {other:?}"),
    }

    // Owner: the plan must cover all 24 hours; the refusal names the uncovered window.
    let mut tx = db.pool.begin().await.unwrap();
    let got = svc
        .save_in(
            &mut tx,
            &branch,
            ORG,
            vec![
                row("A", hm(6, 0), hm(14, 0)),
                row("B", hm(14, 0), hm(22, 0)),
            ],
            Utc::now(),
        )
        .await;
    drop(tx);
    match got {
        Err(AppError::Validation(msg)) => assert_eq!(msg, "shiftPlanInvalid:gap:22:00-06:00"),
        other => panic!("expected shiftPlanInvalid, got {other:?}"),
    }
    assert_eq!(live_names(&db, &branch).await, vec!["A", "B", "C"]);
    db.drop_schema().await;
}

#[tokio::test]
async fn v2_two_shifts_covering_now_preselect_nothing() {
    let db = TestDb::start().await;
    let svc = service(&db);
    let branch = branch_with_plan(&db, &svc).await;
    let mut tx = db.pool.begin().await.unwrap();
    let got = svc
        .choices_in(&mut tx, &branch, bkk(4, 14, 30))
        .await
        .unwrap();
    assert_eq!(
        got.choices
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["A", "B"]
    );
    assert_eq!(got.preselect, None);
    drop(tx);
    db.drop_schema().await;
}

#[tokio::test]
async fn v3_one_shift_covering_now_is_preselected() {
    let db = TestDb::start().await;
    let svc = service(&db);
    let branch = branch_with_plan(&db, &svc).await;
    let mut tx = db.pool.begin().await.unwrap();
    let got = svc
        .choices_in(&mut tx, &branch, bkk(4, 10, 0))
        .await
        .unwrap();
    assert_eq!(got.choices.len(), 1);
    assert_eq!(got.choices[0].name, "A");
    assert_eq!(
        got.preselect.as_deref(),
        Some(got.choices[0].template_id.as_str())
    );
    drop(tx);
    db.drop_schema().await;
}

#[tokio::test]
async fn v4_two_am_offers_only_last_nights_shift() {
    let db = TestDb::start().await;
    let svc = service(&db);
    let branch = branch_with_plan(&db, &svc).await;
    let mut tx = db.pool.begin().await.unwrap();
    let got = svc
        .choices_in(&mut tx, &branch, bkk(5, 2, 0))
        .await
        .unwrap();
    assert_eq!(
        got.choices
            .iter()
            .map(|c| (c.name.as_str(), c.business_date))
            .collect::<Vec<_>>(),
        vec![("C", chrono::NaiveDate::from_ymd_opt(2026, 11, 4).unwrap())]
    );
    assert_eq!(
        got.preselect.as_deref(),
        Some(got.choices[0].template_id.as_str())
    );
    drop(tx);
    db.drop_schema().await;
}

#[tokio::test]
async fn v5_the_plan_uses_the_branchs_timezone() {
    let db = TestDb::start().await;
    let svc = service(&db);
    let branch = branch_with_plan(&db, &svc).await;
    sqlx::query("UPDATE branches SET timezone = 'Asia/Tokyo' WHERE id = $1")
        .bind(branch.as_str())
        .execute(&db.pool)
        .await
        .unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    let plan = svc.plan_in(&mut tx, &branch).await.unwrap();
    assert_eq!(plan.timezone(), chrono_tz::Asia::Tokyo);
    // 10:00 Tokyo = 08:00 Bangkok: A covers it.
    let got = svc
        .choices_in(&mut tx, &branch, bkk(4, 8, 0))
        .await
        .unwrap();
    assert_eq!(
        got.choices
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["A"]
    );
    drop(tx);
    db.drop_schema().await;
}

#[tokio::test]
async fn v6_seed_defaults_is_idempotent_and_save_returns_the_stored_plan() {
    let db = TestDb::start().await;
    let svc = service(&db);
    let branch = db.branch("CS3").await;
    // The `branches_default_shift_templates` trigger already seeded it: seeding again is a no-op.
    assert!(
        !svc.seed_defaults(&branch, ORG).await.unwrap(),
        "a seeded branch is left alone"
    );
    assert_eq!(live_names(&db, &branch).await, vec!["A", "B", "C"]);
    sqlx::query("UPDATE shift_templates SET retired_at = now() WHERE branch_id = $1")
        .bind(branch.as_str())
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        svc.seed_defaults(&branch, ORG).await.unwrap(),
        "first seed inserts"
    );
    assert!(
        !svc.seed_defaults(&branch, ORG).await.unwrap(),
        "second seed is a no-op"
    );
    assert_eq!(live_names(&db, &branch).await, vec!["A", "B", "C"]);

    let mut tx = db.pool.begin().await.unwrap();
    let plan = svc
        .save_in(&mut tx, &branch, ORG, overlapping(), Utc::now())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        plan.templates()
            .iter()
            .map(|t| (t.name.as_str(), t.starts_at))
            .collect::<Vec<_>>(),
        vec![("A", hm(6, 0)), ("B", hm(14, 0)), ("C", hm(22, 0))]
    );
    db.drop_schema().await;
}
