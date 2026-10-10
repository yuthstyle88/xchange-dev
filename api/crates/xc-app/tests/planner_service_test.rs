//! APP-4 (#994) — `PlannerService` against a real PostgreSQL: create, update with
//! `expectedVersion`, a stale version, the owner check on writes and reads, the sign-in claim,
//! and the draft lifetime (owner C4).
//! Written before the implementation by the ticket author's delegate (claude/dell-claude-3). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design `docs/design/blueprint/customer-app.md`: P5 (a draft holds no quote), P6 (exactly one
//! owner; the claim is one statement), P7 (the session's SHA-256 is stored), P13 and §5's note (the
//! lifetime is read only on create; an update keeps `expires_at`; no row or NULL →
//! `appSettingsMissing`), §4.3 (another owner → `outOfScope`; a stale `expected_version` →
//! `staleVersion`, nothing written), §9.1 row APP-4. Ticket #994 §8, §9.
//!
//! Signatures this test assumes (the scaffold commit before this one):
//! - `PlannerService::new(pool)`; `save_in(&self, tx, organization_id: Uuid, owner: &DraftOwner,
//!   save: DraftSave, now) -> Result<PlannerDraft, CustomerAppError>`; `delete_in(&self, tx, owner,
//!   id)`; `get(&self, owner, id)`; `list(&self, owner)`; `claim_session_in(&self, tx, session:
//!   &SessionHash, customer_id: &str, now) -> Result<u64, _>` (all async).
//! - `CustomerAppError::code()` gives the v3 code.
//! - A refused call followed by a COMMIT of its transaction leaves the tables as they were.
//!
//! Not here, because the route seam on `main` cannot carry a customer or an anonymous session
//! until OR-4b #767 merges: the same `Idempotency-Key` replay and `idempotencyConflict` of
//! `plannerDraftSave` (`CommandEndpoint` + `idempotency_records`), the cookie on the first app
//! command, and the view `my_planner_drafts` over HTTP. Those route tests are written once #767
//! has merged (commented on #994).

mod common;

use common::*;
use xc_kernel::money::CurrencyCode;
use xc_kernel::money::CustomerSide::{CustomerBuy, CustomerSell};

#[tokio::test]
async fn create_stores_version_1_and_expires_at_now_plus_the_organisations_lifetime() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());
    let owner = session();

    let mut tx = db.begin().await;
    let draft = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("a new draft is saved");
    tx.commit().await.expect("commit");

    assert_eq!(draft.version, 1);
    assert_eq!(draft.owner, owner);
    assert_eq!(draft.organization_id, org());
    assert_eq!(
        draft.expires_at,
        at(SEVEN_DAYS),
        "created_at + 604 800 s (C4)"
    );
    assert_eq!(draft.created_at, at(0));
    let row = db.draft_row(draft.id).await.expect("the row is stored");
    let DraftOwner::Session(hash) = &owner else {
        unreachable!()
    };
    assert_eq!(row.session_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(row.customer_id, None);
    assert_eq!(row.branch_id.as_deref(), Some(MAIN));
    assert_eq!(row.currency_code, "USD");
    assert_eq!(row.customer_side, "CUSTOMER_BUY");
    assert_eq!(row.quantity.as_deref(), Some("500.0000"));
    assert_eq!(row.budget_thb, None);
    assert_eq!(row.version, 1);
    assert_eq!(row.expires_at, at(SEVEN_DAYS));
    db.finish().await;
}

#[tokio::test]
async fn create_follows_the_lifetime_admin_set_not_a_constant() {
    let db = TestDb::start().await;
    db.set_lifetime(Some(3_600)).await;
    let service = PlannerService::new(db.pool.clone());

    let mut tx = db.begin().await;
    let draft = service
        .save_in(
            &mut tx,
            org(),
            &session(),
            create(plan("USD", CustomerBuy, "1")),
            at(10),
        )
        .await
        .expect("saved");
    tx.commit().await.expect("commit");

    assert_eq!(draft.expires_at, at(10 + 3_600));
    assert_eq!(
        db.draft_row(draft.id).await.expect("stored").expires_at,
        at(10 + 3_600)
    );
    db.finish().await;
}

#[tokio::test]
async fn a_budget_plan_is_stored_with_its_budget_and_no_quantity() {
    let db = TestDb::start().await;
    db.seed_customer("cust-budget").await;
    let service = PlannerService::new(db.pool.clone());
    let mut budget = plan("JPY", CustomerSell, "1");
    budget.amount = DraftAmount::BudgetThb(money("2350.00"));

    let mut tx = db.begin().await;
    let draft = service
        .save_in(
            &mut tx,
            org(),
            &customer("cust-budget"),
            create(budget.clone()),
            at(0),
        )
        .await
        .expect("saved");
    tx.commit().await.expect("commit");

    assert_eq!(draft.plan, budget);
    let row = db.draft_row(draft.id).await.expect("stored");
    assert_eq!(row.customer_id.as_deref(), Some("cust-budget"));
    assert_eq!(row.session_hash, None);
    assert_eq!(row.customer_side, "CUSTOMER_SELL");
    assert_eq!(row.quantity, None);
    assert_eq!(row.budget_thb.as_deref(), Some("2350.00"));
    db.finish().await;
}

#[tokio::test]
async fn update_with_the_current_version_writes_the_plan_and_bumps_the_version() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());
    let owner = session();
    let mut tx = db.begin().await;
    let first = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");

    let mut tx = db.begin().await;
    let second = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            update(first.id, 1, plan("JPY", CustomerSell, "10000")),
            at(60),
        )
        .await
        .expect("updated at the current version");
    tx.commit().await.expect("commit");

    assert_eq!(second.id, first.id);
    assert_eq!(second.version, 2);
    assert_eq!(second.updated_at, at(60));
    assert_eq!(second.created_at, at(0));
    let row = db.draft_row(first.id).await.expect("stored");
    assert_eq!(row.version, 2);
    assert_eq!(row.currency_code, "JPY");
    assert_eq!(row.customer_side, "CUSTOMER_SELL");
    assert_eq!(row.quantity.as_deref(), Some("10000.0000"));
    assert_eq!(row.updated_at, at(60));
    assert_eq!(db.drafts().await, 1, "an update writes no second row");
    db.finish().await;
}

#[tokio::test]
async fn update_keeps_expires_at_even_after_the_lifetime_changes() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());
    let owner = session();
    let mut tx = db.begin().await;
    let first = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");
    db.set_lifetime(Some(60)).await;

    let mut tx = db.begin().await;
    let second = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            update(first.id, 1, plan("USD", CustomerBuy, "600")),
            at(3_000),
        )
        .await
        .expect("updated");
    tx.commit().await.expect("commit");

    assert_eq!(
        second.expires_at,
        at(SEVEN_DAYS),
        "an update keeps expires_at"
    );
    assert_eq!(
        db.draft_row(first.id).await.expect("stored").expires_at,
        at(SEVEN_DAYS)
    );
    db.finish().await;
}

#[tokio::test]
async fn update_keeps_expires_at_even_when_the_settings_row_is_gone() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());
    let owner = session();
    let mut tx = db.begin().await;
    let first = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");
    db.delete_settings().await;

    let mut tx = db.begin().await;
    let second = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            update(first.id, 1, plan("USD", CustomerBuy, "700")),
            at(5),
        )
        .await
        .expect("an update does not read the settings (§5: read only on create)");
    tx.commit().await.expect("commit");

    assert_eq!(second.version, 2);
    assert_eq!(second.expires_at, at(SEVEN_DAYS));
    db.finish().await;
}

#[tokio::test]
async fn update_with_a_stale_version_is_stale_version_and_writes_nothing() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());
    let owner = session();
    let mut tx = db.begin().await;
    let first = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");
    let mut tx = db.begin().await;
    service
        .save_in(
            &mut tx,
            org(),
            &owner,
            update(first.id, 1, plan("USD", CustomerBuy, "600")),
            at(10),
        )
        .await
        .expect("version 1 → 2");
    tx.commit().await.expect("commit");
    let before = db.draft_row(first.id).await.expect("stored");

    let mut tx = db.begin().await;
    let refused = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            update(first.id, 1, plan("EUR", CustomerSell, "9")),
            at(20),
        )
        .await
        .expect_err("version 1 is stale");
    tx.commit().await.expect("commit after the refusal");

    assert_eq!(refused.code(), "staleVersion");
    assert_eq!(
        db.draft_row(first.id).await.expect("stored"),
        before,
        "nothing written"
    );
    db.finish().await;
}

#[tokio::test]
async fn update_of_another_sessions_draft_is_out_of_scope_and_writes_nothing() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());
    let mut tx = db.begin().await;
    let first = service
        .save_in(
            &mut tx,
            org(),
            &session(),
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");
    let before = db.draft_row(first.id).await.expect("stored");

    let mut tx = db.begin().await;
    let refused = service
        .save_in(
            &mut tx,
            org(),
            &session(),
            update(first.id, 1, plan("USD", CustomerBuy, "1")),
            at(5),
        )
        .await
        .expect_err("another session");
    tx.commit().await.expect("commit after the refusal");

    assert_eq!(refused.code(), "outOfScope");
    assert_eq!(db.draft_row(first.id).await.expect("stored"), before);
    db.finish().await;
}

#[tokio::test]
async fn update_of_a_customers_draft_by_a_session_is_out_of_scope() {
    let db = TestDb::start().await;
    db.seed_customer("cust-a").await;
    let service = PlannerService::new(db.pool.clone());
    let mut tx = db.begin().await;
    let first = service
        .save_in(
            &mut tx,
            org(),
            &customer("cust-a"),
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");
    let before = db.draft_row(first.id).await.expect("stored");

    let mut tx = db.begin().await;
    let refused = service
        .save_in(
            &mut tx,
            org(),
            &session(),
            update(first.id, 1, plan("USD", CustomerBuy, "1")),
            at(5),
        )
        .await
        .expect_err("a session is not the customer");
    tx.commit().await.expect("commit after the refusal");

    assert_eq!(refused.code(), "outOfScope");
    assert_eq!(db.draft_row(first.id).await.expect("stored"), before);
    db.finish().await;
}

#[tokio::test]
async fn update_of_a_missing_draft_is_not_found() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());

    let mut tx = db.begin().await;
    let refused = service
        .save_in(
            &mut tx,
            org(),
            &session(),
            update(uuid::Uuid::new_v4(), 1, plan("USD", CustomerBuy, "1")),
            at(0),
        )
        .await
        .expect_err("no such draft");
    tx.commit().await.expect("commit after the refusal");

    assert_eq!(refused.code(), "notFound");
    assert_eq!(db.drafts().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn delete_by_the_owner_removes_the_draft() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());
    let owner = session();
    let mut tx = db.begin().await;
    let draft = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");

    let mut tx = db.begin().await;
    service
        .delete_in(&mut tx, &owner, draft.id)
        .await
        .expect("the owner deletes it");
    tx.commit().await.expect("commit");

    assert_eq!(db.draft_row(draft.id).await, None);
    db.finish().await;
}

#[tokio::test]
async fn delete_by_another_owner_is_out_of_scope_and_keeps_the_draft() {
    let db = TestDb::start().await;
    db.seed_customer("cust-b").await;
    let service = PlannerService::new(db.pool.clone());
    let mut tx = db.begin().await;
    let draft = service
        .save_in(
            &mut tx,
            org(),
            &session(),
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");

    for intruder in [session(), customer("cust-b")] {
        let mut tx = db.begin().await;
        let refused = service
            .delete_in(&mut tx, &intruder, draft.id)
            .await
            .expect_err("not the owner");
        tx.commit().await.expect("commit after the refusal");
        assert_eq!(refused.code(), "outOfScope", "{intruder:?}");
    }
    assert!(db.draft_row(draft.id).await.is_some(), "the draft is kept");
    db.finish().await;
}

#[tokio::test]
async fn a_session_cannot_read_a_customers_draft() {
    let db = TestDb::start().await;
    db.seed_customer("cust-c").await;
    let service = PlannerService::new(db.pool.clone());
    let mut tx = db.begin().await;
    let draft = service
        .save_in(
            &mut tx,
            org(),
            &customer("cust-c"),
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");

    let refused = service
        .get(&session(), draft.id)
        .await
        .expect_err("not the session's");
    assert_eq!(refused.code(), "outOfScope");
    let mine = service
        .get(&customer("cust-c"), draft.id)
        .await
        .expect("the customer reads it");
    assert_eq!(mine.id, draft.id);
    db.finish().await;
}

#[tokio::test]
async fn a_session_cannot_read_another_sessions_draft() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());
    let owner = session();
    let mut tx = db.begin().await;
    let draft = service
        .save_in(
            &mut tx,
            org(),
            &owner,
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");

    let refused = service
        .get(&session(), draft.id)
        .await
        .expect_err("another session");
    assert_eq!(refused.code(), "outOfScope");
    assert_eq!(
        service.get(&owner, draft.id).await.expect("its own").id,
        draft.id
    );
    db.finish().await;
}

#[tokio::test]
async fn a_customer_cannot_read_a_sessions_or_another_customers_draft() {
    let db = TestDb::start().await;
    db.seed_customer("cust-d").await;
    db.seed_customer("cust-e").await;
    let service = PlannerService::new(db.pool.clone());
    let mut tx = db.begin().await;
    let of_session = service
        .save_in(
            &mut tx,
            org(),
            &session(),
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    let of_d = service
        .save_in(
            &mut tx,
            org(),
            &customer("cust-d"),
            create(plan("USD", CustomerBuy, "5")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");

    for id in [of_session.id, of_d.id] {
        let refused = service
            .get(&customer("cust-e"), id)
            .await
            .expect_err("not cust-e's");
        assert_eq!(refused.code(), "outOfScope");
    }
    db.finish().await;
}

#[tokio::test]
async fn reading_a_missing_draft_is_not_found() {
    let db = TestDb::start().await;
    let service = PlannerService::new(db.pool.clone());

    let refused = service
        .get(&session(), uuid::Uuid::new_v4())
        .await
        .expect_err("none");

    assert_eq!(refused.code(), "notFound");
    db.finish().await;
}

#[tokio::test]
async fn list_answers_only_the_owners_drafts() {
    let db = TestDb::start().await;
    db.seed_customer("cust-f").await;
    let service = PlannerService::new(db.pool.clone());
    let mine = session();
    let other = session();
    let mut tx = db.begin().await;
    let a = service
        .save_in(
            &mut tx,
            org(),
            &mine,
            create(plan("USD", CustomerBuy, "1")),
            at(0),
        )
        .await
        .expect("created");
    let b = service
        .save_in(
            &mut tx,
            org(),
            &mine,
            create(plan("JPY", CustomerSell, "2")),
            at(1),
        )
        .await
        .expect("created");
    service
        .save_in(
            &mut tx,
            org(),
            &other,
            create(plan("EUR", CustomerBuy, "3")),
            at(2),
        )
        .await
        .expect("created");
    service
        .save_in(
            &mut tx,
            org(),
            &customer("cust-f"),
            create(plan("GBP", CustomerBuy, "4")),
            at(3),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");

    let mut ids: Vec<_> = service
        .list(&mine)
        .await
        .expect("listed")
        .into_iter()
        .map(|d| d.id)
        .collect();
    ids.sort();
    let mut expected = vec![a.id, b.id];
    expected.sort();
    assert_eq!(ids, expected);
    let of_f = service.list(&customer("cust-f")).await.expect("listed");
    assert_eq!(of_f.len(), 1);
    assert_eq!(
        of_f[0].plan.currency,
        CurrencyCode::parse("GBP").expect("GBP")
    );
    assert!(service.list(&session()).await.expect("listed").is_empty());
    db.finish().await;
}

#[tokio::test]
async fn claim_moves_every_session_draft_to_the_customer_and_leaves_other_sessions_alone() {
    let db = TestDb::start().await;
    db.seed_customer("cust-g").await;
    let service = PlannerService::new(db.pool.clone());
    let id = AppSessionId::generate();
    let mine = DraftOwner::Session(id.hash());
    let other = session();
    let mut tx = db.begin().await;
    let a = service
        .save_in(
            &mut tx,
            org(),
            &mine,
            create(plan("USD", CustomerBuy, "1")),
            at(0),
        )
        .await
        .expect("created");
    let b = service
        .save_in(
            &mut tx,
            org(),
            &mine,
            create(plan("JPY", CustomerSell, "2")),
            at(1),
        )
        .await
        .expect("created");
    let c = service
        .save_in(
            &mut tx,
            org(),
            &other,
            create(plan("EUR", CustomerBuy, "3")),
            at(2),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");

    let mut tx = db.begin().await;
    let moved = service
        .claim_session_in(&mut tx, &id.hash(), "cust-g", at(100))
        .await
        .expect("claimed");
    tx.commit().await.expect("commit");

    assert_eq!(moved, 2);
    for draft in [a.id, b.id] {
        let row = db.draft_row(draft).await.expect("kept");
        assert_eq!(row.customer_id.as_deref(), Some("cust-g"));
        assert_eq!(row.session_hash, None, "exactly one owner after the claim");
    }
    let untouched = db.draft_row(c.id).await.expect("kept");
    assert_eq!(untouched.customer_id, None);
    assert!(untouched.session_hash.is_some());
    assert_eq!(
        service
            .list(&customer("cust-g"))
            .await
            .expect("listed")
            .len(),
        2
    );
    assert!(service.list(&mine).await.expect("listed").is_empty());
    db.finish().await;
}

#[tokio::test]
async fn a_second_claim_moves_none() {
    let db = TestDb::start().await;
    db.seed_customer("cust-h").await;
    let service = PlannerService::new(db.pool.clone());
    let id = AppSessionId::generate();
    let mut tx = db.begin().await;
    service
        .save_in(
            &mut tx,
            org(),
            &DraftOwner::Session(id.hash()),
            create(plan("USD", CustomerBuy, "1")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");
    let mut tx = db.begin().await;
    assert_eq!(
        service
            .claim_session_in(&mut tx, &id.hash(), "cust-h", at(1))
            .await
            .expect("first"),
        1
    );
    tx.commit().await.expect("commit");

    let mut tx = db.begin().await;
    let again = service
        .claim_session_in(&mut tx, &id.hash(), "cust-h", at(2))
        .await
        .expect("second");
    tx.commit().await.expect("commit");

    assert_eq!(again, 0);
    assert_eq!(db.drafts().await, 1);
    db.finish().await;
}

#[tokio::test]
async fn create_without_a_settings_row_is_app_settings_missing_and_writes_nothing() {
    let db = TestDb::start().await;
    db.delete_settings().await;
    let service = PlannerService::new(db.pool.clone());

    let mut tx = db.begin().await;
    let refused = service
        .save_in(
            &mut tx,
            org(),
            &session(),
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect_err("no settings row");
    tx.commit().await.expect("commit after the refusal");

    assert_eq!(refused, CustomerAppError::AppSettingsMissing);
    assert_eq!(refused.code(), "appSettingsMissing");
    assert_eq!(db.drafts().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn create_with_a_null_lifetime_is_app_settings_missing_and_writes_nothing() {
    let db = TestDb::start().await;
    db.set_lifetime(None).await;
    let service = PlannerService::new(db.pool.clone());

    let mut tx = db.begin().await;
    let refused = service
        .save_in(
            &mut tx,
            org(),
            &session(),
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect_err("a NULL lifetime");
    tx.commit().await.expect("commit after the refusal");

    assert_eq!(refused.code(), "appSettingsMissing");
    assert_eq!(db.drafts().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn a_draft_holds_no_quote() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;
    let service = PlannerService::new(db.pool.clone());
    let before = db.untouchable_counts().await;

    let mut tx = db.begin().await;
    service
        .save_in(
            &mut tx,
            org(),
            &session(),
            create(plan("USD", CustomerBuy, "500")),
            at(0),
        )
        .await
        .expect("created");
    tx.commit().await.expect("commit");

    assert_eq!(
        db.untouchable_counts().await,
        before,
        "P5: saving a draft makes no quote, stock or booking row"
    );
    db.finish().await;
}
