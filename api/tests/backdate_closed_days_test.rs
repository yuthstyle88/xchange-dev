mod quote_common;
mod tx4_common;

use chrono::NaiveDate;
use quote_common::ORG_1;
use tx4_common::*;
use xc_kernel::ids::BranchId;
use xchange::infrastructure::finance::closed_days::ClosedDays;

#[actix_web::test]
async fn certified_date_for_branch_is_closed() {
    let desk = Desk::start().await;
    let date = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
    desk.db
        .exec(&format!(
            "INSERT INTO reconciliation_certifications (id, organization_id, branch_id, business_date, \
                 figures_hash, figures, certified_by, certified_at) \
             VALUES (gen_random_uuid(), '{ORG_1}', 'MAIN', '{date}', 'h', '{{}}', gen_random_uuid(), now())"
        ))
        .await;

    let mut tx = desk.db.pool.begin().await.unwrap();
    let is_closed = ClosedDays::is_closed_in(&mut tx, &BranchId::new("MAIN"), date)
        .await
        .expect("check closed day");
    tx.commit().await.unwrap();

    assert!(is_closed);
    desk.db.finish().await;
}

#[actix_web::test]
async fn uncertified_date_or_branch_is_not_closed() {
    let desk = Desk::start().await;
    let certified_date = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
    let other_date = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
    desk.db
        .exec(&format!(
            "INSERT INTO reconciliation_certifications (id, organization_id, branch_id, business_date, \
                 figures_hash, figures, certified_by, certified_at) \
             VALUES (gen_random_uuid(), '{ORG_1}', 'MAIN', '{certified_date}', 'h', '{{}}', gen_random_uuid(), now())"
        ))
        .await;

    let mut tx = desk.db.pool.begin().await.unwrap();
    let closed_other_date = ClosedDays::is_closed_in(&mut tx, &BranchId::new("MAIN"), other_date)
        .await
        .expect("check other date");
    let closed_other_branch =
        ClosedDays::is_closed_in(&mut tx, &BranchId::new("BR2"), certified_date)
            .await
            .expect("check other branch");
    tx.commit().await.unwrap();

    assert!(!closed_other_date);
    assert!(!closed_other_branch);
    desk.db.finish().await;
}
