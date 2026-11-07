//! F50a (`docs/design/backdated-keying.md` §16.5; ticket #6713):
//! `views/transaction` returns `transaction.notes` trimmed, or absent when blank.

mod common;
mod quote_common;

use quote_common::{ORG_1, TestDb};
use uuid::Uuid;
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::presentation::dto::v3::transaction_detail::TransactionFilter;
use xchange::presentation::http::v3::transaction::TransactionView;
use xchange::presentation::http::v3::view::BranchView;
use xchange::presentation::test_callers;

const T0: &str = "2026-10-06T05:00:00Z";

async fn seed_tx(db: &TestDb, notes: &str) -> String {
    db.exec(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'CU-MAIN', organization_id, 'CU-MAIN', 'Somchai', 'DEMO', now() \
         FROM branches WHERE id = 'MAIN' ON CONFLICT (id) DO NOTHING",
    )
    .await;
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, submission_id, payload_hash, \
            customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, \
            paid_at, recorded_at, recorded_by, status, organization_id) \
         VALUES ($1::uuid, NULL, 'MAIN', 'DEMO', $2, 'hash', 'CU-MAIN', 'Somchai', 'THA', \
            'EMP-001', 'WALK-IN', $3, '2026-10-06', $4::timestamptz, $4::timestamptz, 'owner', \
            'DRAFT', $5::uuid)",
    )
    .bind(&id)
    .bind(format!("sub-{id}"))
    .bind(notes)
    .bind(T0)
    .bind(ORG_1)
    .execute(&db.pool)
    .await
    .expect("insert transaction");
    id
}

fn actor() -> xc_auth::domain::access::Actor {
    test_callers::staff("emp-1", &[Role::BranchManager], BranchId::new("MAIN"))
}

#[tokio::test]
async fn note_with_text_comes_back_trimmed() {
    let db = TestDb::start().await;
    let id = seed_tx(&db, "  #48 teller Somying  ").await;
    let view = TransactionView {
        pool: db.pool.clone(),
    };
    let data = view
        .load(
            &BranchId::new("MAIN"),
            &actor(),
            &TransactionFilter { id: Some(id) },
        )
        .await
        .expect("load transaction");
    let detail = data.transaction.expect("transaction exists");
    assert_eq!(detail.notes.as_deref(), Some("#48 teller Somying"));
    let json = serde_json::to_value(&detail).expect("serialize detail");
    assert_eq!(json["notes"], "#48 teller Somying");
    db.finish().await;
}

#[tokio::test]
async fn blank_note_is_absent() {
    let db = TestDb::start().await;
    let empty_id = seed_tx(&db, "").await;
    let ws_id = seed_tx(&db, "    ").await;
    let view = TransactionView {
        pool: db.pool.clone(),
    };
    for id in [empty_id, ws_id] {
        let data = view
            .load(
                &BranchId::new("MAIN"),
                &actor(),
                &TransactionFilter { id: Some(id) },
            )
            .await
            .expect("load transaction");
        let detail = data.transaction.expect("transaction exists");
        assert_eq!(detail.notes, None);
        let json = serde_json::to_value(&detail).expect("serialize detail");
        assert_eq!(json.get("notes"), None);
    }
    db.finish().await;
}
