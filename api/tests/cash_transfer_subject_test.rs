//! F48a up-front tests (`docs/design/backdated-keying.md` §16.2): `load_cash_transfer_subjects` gives a `CASH_TRANSFER`
//! approval request the transfer's totals per currency, its note and its branches' names, so the approval card can show
//! the figure the approver decides on. Day 4 on xc-v3: the approver saw only "#c35925fb · MAIN · เกินวงเงิน" for a
//! THB 150,000 top-up (`Approvals.tsx` `cashTransferDetail` reads only the payload's id, branch and limit flag).
//!
//! Written before the implementation by the sub-controller claude/3b81c9d5. Do not edit, delete or weaken; add tests in
//! a file of your own. Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod quote_common;

use quote_common::{ORG_1, TestDb};
use sqlx::Row;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xchange::presentation::dto::v3::approval_requests::{
    CashTransferSubjectLine, load_cash_transfer_subjects,
};

const BR2: &str = "BR2";

async fn person(db: &TestDb) -> String {
    let mut conn = db.pool.acquire().await.expect("a connection");
    xc_testdb::people::seed_person(&mut conn, ORG_1, "Finance Ploy", &[])
        .await
        .expect("seed a person")
}

/// A DRAFT transfer with `lines` of `(currency, face value, qty_sent)`.
async fn transfer(
    db: &TestDb,
    by: &str,
    kind: &str,
    from: &str,
    to: Option<&str>,
    note: Option<&str>,
    lines: &[(&str, &str, i32)],
) -> Uuid {
    let id: Uuid = sqlx::query(
        "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, to_branch_id, status, created_by, note, \
         idempotency_key) VALUES ($1::uuid, $2, $3, $4, 'DRAFT', $5::uuid, $6, $7) RETURNING id",
    )
    .bind(ORG_1)
    .bind(kind)
    .bind(from)
    .bind(to)
    .bind(by)
    .bind(note)
    .bind(Uuid::new_v4().to_string())
    .fetch_one(&db.pool)
    .await
    .expect("insert a transfer")
    .get(0);
    for (currency, face, qty) in lines {
        sqlx::query(
            "INSERT INTO cash_transfer_lines (transfer_id, organization_id, currency, denomination_id, qty_sent) \
             SELECT $1, $2::uuid, $3, d.id, $5 FROM denominations d \
              WHERE d.currency_code = $3 AND d.face_value = $4::numeric \
                AND (d.organization_id = $2::uuid OR d.organization_id IS NULL) \
              ORDER BY d.organization_id NULLS LAST LIMIT 1",
        )
        .bind(id)
        .bind(ORG_1)
        .bind(*currency)
        .bind(*face)
        .bind(*qty)
        .execute(&db.pool)
        .await
        .expect("insert a transfer line");
    }
    id
}

fn line(currency: &str, amount: &str) -> CashTransferSubjectLine {
    CashTransferSubjectLine {
        currency: currency.into(),
        amount: amount.into(),
    }
}

#[tokio::test]
async fn f48a_1_a_top_up_carries_its_totals_note_and_branch_name() {
    let db = TestDb::start().await;
    let by = person(&db).await;
    // THB 1000 × 150 = 150000; USD 100 × 5 + USD 20 × 1 = 520.
    let id = transfer(
        &db,
        &by,
        "EXTERNAL_IN",
        "MAIN",
        None,
        Some("  ยอดสมมติ — เติมเงินต้นวัน  "),
        &[("THB", "1000", 150), ("USD", "100", 5), ("USD", "20", 1)],
    )
    .await;

    let got = load_cash_transfer_subjects(&db.pool, &BranchId::new("MAIN"), &[id])
        .await
        .expect("load the subjects");
    let s = got.get(&id).expect("the transfer's subject");
    assert_eq!(s.transfer_id, id.to_string());
    assert_eq!(s.kind, "EXTERNAL_IN");
    assert_eq!(s.from_branch_id, "MAIN");
    assert_eq!(
        s.from_branch_name.as_deref(),
        Some("XC-B"),
        "branches.name, not the id"
    );
    assert_eq!(s.to_branch_name, None);
    assert_eq!(s.note.as_deref(), Some("ยอดสมมติ — เติมเงินต้นวัน"), "trimmed");
    assert_eq!(
        s.lines,
        vec![line("THB", "150000.00"), line("USD", "520.00")],
        "one total per currency, ordered by code, at the currency's minor unit"
    );
    db.finish().await;
}

#[tokio::test]
async fn f48a_2_a_branch_transfer_names_both_branches_and_a_blank_note_is_none() {
    let db = TestDb::start().await;
    db.exec(&format!(
        "INSERT INTO branches (id, name, organization_id) VALUES ('{BR2}', 'Silom #2', '{ORG_1}')"
    ))
    .await;
    let by = person(&db).await;
    let id = transfer(
        &db,
        &by,
        "BRANCH",
        "MAIN",
        Some(BR2),
        Some("   "),
        &[("THB", "500", 4)],
    )
    .await;

    let got = load_cash_transfer_subjects(&db.pool, &BranchId::new("MAIN"), &[id])
        .await
        .expect("load the subjects");
    let s = got.get(&id).expect("the transfer's subject");
    assert_eq!(s.kind, "BRANCH");
    assert_eq!(s.to_branch_name.as_deref(), Some("Silom #2"));
    assert_eq!(s.note, None, "a blank note is no note");
    assert_eq!(s.lines, vec![line("THB", "2000.00")]);
    db.finish().await;
}

#[tokio::test]
async fn f48a_3_a_transfer_leaving_another_branch_or_unknown_is_absent() {
    let db = TestDb::start().await;
    db.exec(&format!(
        "INSERT INTO branches (id, name, organization_id) VALUES ('{BR2}', 'Silom #2', '{ORG_1}')"
    ))
    .await;
    let by = person(&db).await;
    let other = transfer(
        &db,
        &by,
        "EXTERNAL_IN",
        BR2,
        None,
        None,
        &[("THB", "1000", 1)],
    )
    .await;
    let unknown = Uuid::new_v4();

    let got = load_cash_transfer_subjects(&db.pool, &BranchId::new("MAIN"), &[other, unknown])
        .await
        .expect("load the subjects");
    assert!(got.is_empty(), "nothing for MAIN: {got:?}");

    let none = load_cash_transfer_subjects(&db.pool, &BranchId::new("MAIN"), &[])
        .await
        .expect("an empty list is fine");
    assert!(none.is_empty());
    db.finish().await;
}
