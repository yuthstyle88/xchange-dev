//! LY-3 #1385 up-front tests: completing or reversing an exchange writes a loyalty event
//! (design `docs/design/blueprint/membership-points.md` §9.3 row LY-3, D-4, D-5, §6.2).
//!
//! They go through XS-0's public seam only, `bootstrap::exchange_effects::effects(&state)`, with a test
//! `CompleteExchange` / `PostReversal` stand-in (§9.3: "through XS-0, with a test `CompleteExchange` stand-in").
//! Nothing here names `bootstrap/exchange_effects/loyalty.rs` or its types: the file is found by XS-0's
//! generator, and no line in `complete.rs` or `reversal_post.rs` is needed (§9.1).
//!
//! What they lock:
//! - a completed LIVE exchange whose customer has an ACTIVE verified PHONE leaves one `loyalty.exchange_completed`
//!   row (key `exchange:<transaction id>`, aggregate `transaction/<transaction id>`, payload `{transactionId}`),
//!   written at the `Outbox` phase, not at `Redeem` (§6.2: `LoyaltyEffect`, phase `Outbox`);
//! - a completion that rolls back after the Outbox step (for example `stockInsufficient`) leaves none;
//! - replaying the same completion leaves one row (OB-1's key);
//! - D-5: a DEMO exchange, an exchange with no customer, and a customer without an ACTIVE verified PHONE leave none;
//! - a posted reversal of a sent completion leaves one `loyalty.exchange_reversed` row (key `reversal:<reversal id>`,
//!   aggregate = the ORIGINAL transaction, payload `{reversalId}`); a reversal whose completion sent nothing leaves
//!   none.
//!
//! - the lane controller's rulings on #1385 (claude/ad05bf8d): `effect` always registers (Q1 A); a reversal is sent
//!   only if its completion's `loyalty.exchange_completed` row exists (Q2 A), so a phone verified AFTER the completion
//!   does not make its reversal sent; the compat path joins TX-6's tests (Q3).
//!
//! Every completion here is a real COMPLETED LIVE exchange (TX-5's `Counter`: rows, lines and receipt revision 1),
//! and every reversal a real POSTED `reversals` row with its REVERSAL receipt, because the effect reads
//! `document_ref` / `lines` from the transaction and `full` from the reversal in the same transaction (ruling on the
//! facts gap). Those three values are not in the stored row (the payload is references only, D-4), so the tests
//! cannot pin them; they pin that the effect works on real rows.
//!
//! Every "leaves none" case runs next to a case that leaves one row, and asserts the exact total, so no test passes
//! on a seam that runs nothing.
//!
//! Written before the implementation by the up-front test writer (claude/07d3765f), ordered by the lane controller
//! claude/ad05bf8d. Do not edit, delete or weaken; add tests in a file of your own. The database tests need
//! `TEST_DATABASE_URL` naming a `_test` database (never port 6381); nothing skips.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::Arc;

use pos_error::{AppError, AppResult};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::Value;
use tx4_common::*;
use tx5_common::*;
use uuid::Uuid;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_kernel::mode::Mode;
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::effects::{
    EffectPhase, ExchangeCompletedFacts, ExchangeEffects, PostedReversal,
};
use xchange::bootstrap::AppState;
use xchange::bootstrap::exchange_effects;

// ---------------------------------------------------------------- the room

/// TX-5's `Counter` (a schema of its own, MAIN's open shift, real LIVE completions), plus the `AppState` that XS-0's
/// `effects(&state)` is asked with (`common::settings(&[])`: no Loyalty setting, ruling Q1 A).
struct Room {
    c: Counter,
    state: AppState,
    ledger: Ledger,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_default();
        assert!(!url.contains(":6381"), "never port 6381");
        let c = Counter::start().await;
        let state = AppState::new(Arc::new(common::settings(&[])), c.desk.db.pool.clone());
        Room {
            c,
            state,
            ledger: Ledger::new("3210.00", None),
        }
    }

    async fn finish(self) {
        self.c.finish().await;
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.c.desk.db.pool
    }

    /// The registered effects, exactly as the completion and the reversal get them.
    fn effects(&self) -> ExchangeEffects {
        exchange_effects::effects(&self.state)
    }

    /// A real COMPLETED LIVE exchange (100 USD bought at MAIN, receipt revision 1), and the facts TX-5 hands its
    /// effects for it. `customer` / `mode` are the facts' values; the stored row keeps `tx4_common::CUSTOMER`.
    async fn sale(&self, customer: Option<&str>, mode: Mode) -> ExchangeCompletedFacts {
        let now = whole_second_now();
        let t = self.c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
        let done = self
            .c
            .complete(&self.ledger, t.id, later(now, 5))
            .await
            .unwrap_or_else(|e| panic!("TX-5 completes the exchange: {e}"));
        let completed_at = done
            .transaction
            .completed_at
            .expect("a COMPLETED exchange has completed_at");
        ExchangeCompletedFacts {
            organization_id: done.transaction.organization_id,
            branch_id: done.transaction.branch_id.clone(),
            transaction_id: done.transaction.id.0,
            quote_id: done.transaction.quote_id.expect("a quoted exchange"),
            customer_id: customer.map(str::to_owned),
            mode,
            completed_at,
            business_date: done.transaction.value_date,
            reversible_until: completed_at + chrono::Duration::hours(24),
        }
    }

    /// A customer of the organisation with no identity.
    async fn customer(&self, org: Uuid) -> String {
        let id = unique("CX-LY3");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2, $1, 'Loyalty Test', 'LIVE', now())",
        )
        .bind(&id)
        .bind(org)
        .execute(self.pool())
        .await
        .expect("insert a customer");
        id
    }

    /// A PHONE identity of `customer`: ACTIVE or REVOKED, verified or not. The D-5 check never decrypts, so the
    /// stored value is opaque bytes.
    async fn phone(&self, org: Uuid, customer: &str, status: &str, verified: bool) {
        sqlx::query(
            "INSERT INTO customer_identities ( \
                 id, customer_id, organization_id, type, encrypted_value, key_id, \
                 lookup_hash, verified_at, status, created_at, updated_at \
             ) VALUES ($1, $2, $3, 'PHONE', $4, 'k1', $5, $6, $7, now(), now())",
        )
        .bind(Uuid::new_v4())
        .bind(customer)
        .bind(org)
        .bind(unique("cipher").into_bytes())
        .bind(unique("lookup").into_bytes())
        .bind(verified.then(|| whole_second_now() - chrono::Duration::days(1)))
        .bind(status)
        .execute(self.pool())
        .await
        .expect("insert a PHONE identity");
    }

    /// `tx4_common::CUSTOMER` with an ACTIVE verified PHONE (D-5's "sent" case).
    async fn member(&self, org: Uuid) -> String {
        self.phone(org, CUSTOMER, "ACTIVE", true).await;
        CUSTOMER.to_owned()
    }

    /// A real POSTED reversal of `e`'s transaction (RV-1's `reversals` row with its approval request and its REVERSAL
    /// receipt, revision 2), and the facts RV-3 hands its effects for it. `posting_id` is one of the shift's real
    /// ledger postings, so the row stays valid when RV-3 adds that foreign key.
    async fn reversal(&self, e: &ExchangeCompletedFacts) -> PostedReversal {
        let pool = self.pool();
        let reversal_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let receipt_id = Uuid::new_v4();
        let posted_at = e.completed_at + chrono::Duration::minutes(30);
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, \
                 policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES ($1, $2, 'TRANSACTION_REVERSAL', $3, $4, 'hash', 'v1', '{}'::jsonb, $5, $6, 'APPROVED', $7)",
        )
        .bind(request_id)
        .bind(e.organization_id)
        .bind(e.branch_id.as_str())
        .bind(reversal_id.to_string())
        .bind(TELLER)
        .bind(posted_at)
        .bind(unique("rev-idem"))
        .execute(pool)
        .await
        .expect("the reversal's approval request");
        sqlx::query(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, document_ref, \
                 created_at, updated_at, kind) \
             SELECT $1, organization_id, transaction_id, number || '-R', 2, $2, document_ref, $2, $2, 'REVERSAL' \
             FROM receipts WHERE transaction_id = $3 AND revision = 1",
        )
        .bind(receipt_id)
        .bind(posted_at)
        .bind(e.transaction_id)
        .execute(pool)
        .await
        .expect("the reversal's receipt, revision 2");
        let posting_id: Uuid = sqlx::query_scalar("SELECT id FROM ledger_postings LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("a real ledger posting (the shift's funding)");
        let amount: Decimal = sqlx::query_scalar(
            "SELECT COALESCE(sum(settle_thb), 0) FROM transaction_lines WHERE transaction_id = $1",
        )
        .bind(e.transaction_id)
        .fetch_one(pool)
        .await
        .expect("the original's settled THB");
        sqlx::query(
            "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, reason, \
                 amount_thb, business_date, approval_request_id, posting_id, receipt_id, shift_id, requested_by, \
                 requested_at, posted_by, posted_at) \
             VALUES ($1, $2, $3, $4, 'POSTED', 'customer changed their mind', $5, $6, $7, $8, $9, $10, $11, $12, \
                 $11, $13)",
        )
        .bind(reversal_id)
        .bind(e.organization_id)
        .bind(e.branch_id.as_str())
        .bind(e.transaction_id)
        .bind(amount)
        .bind(e.business_date)
        .bind(request_id)
        .bind(posting_id)
        .bind(receipt_id)
        .bind(&self.c.shift)
        .bind(TELLER)
        .bind(e.completed_at + chrono::Duration::minutes(10))
        .bind(posted_at)
        .execute(pool)
        .await
        .expect("a POSTED reversal");
        PostedReversal {
            organization_id: e.organization_id,
            reversal_id,
            transaction_id: e.transaction_id,
            customer_id: e.customer_id.clone(),
            mode: e.mode,
            posted_at,
            amount_thb: amount,
        }
    }

    async fn rows(&self) -> Vec<OutboxRow> {
        sqlx::query_as(
            "SELECT organization_id, event_type, destination, aggregate_type, aggregate_id, \
                    idempotency_key, payload_redacted \
             FROM outbox_events WHERE destination = 'loyalty' ORDER BY created_at, idempotency_key",
        )
        .fetch_all(self.pool())
        .await
        .expect("read loyalty outbox rows")
    }
}

#[derive(Debug, sqlx::FromRow)]
struct OutboxRow {
    organization_id: Uuid,
    event_type: String,
    destination: String,
    aggregate_type: String,
    aggregate_id: String,
    idempotency_key: String,
    payload_redacted: Value,
}

// ---------------------------------------------------------------- the stand-ins

/// How the stand-in completion ends after its effect steps.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ending {
    Commit,
    /// A later step refuses (for example `stockInsufficient`) and the whole transaction rolls back.
    RollBack,
}

/// TX-5's `CompleteExchange::run_in` reduced to its two effect steps (§09: *redeem benefit*, then *write outbox*), in
/// one transaction.
async fn complete(room: &Room, e: &ExchangeCompletedFacts, ending: Ending) -> AppResult<()> {
    let effects = room.effects();
    let mut tx = room.pool().begin().await.map_err(AppError::Database)?;
    effects
        .run_completed_in(&mut tx, EffectPhase::Redeem, e)
        .await?;
    effects
        .run_completed_in(&mut tx, EffectPhase::Outbox, e)
        .await?;
    match ending {
        Ending::Commit => {
            tx.commit().await.map_err(AppError::Database)?;
            Ok(())
        }
        Ending::RollBack => {
            tx.rollback().await.map_err(AppError::Database)?;
            Err(AppError::Domain("stockInsufficient".to_owned()))
        }
    }
}

/// RV-3's `PostReversal` reduced to its *member recalculation event* step (§09), in one transaction.
async fn post_reversal(room: &Room, r: &PostedReversal) -> AppResult<()> {
    let effects = room.effects();
    let mut tx = room.pool().begin().await.map_err(AppError::Database)?;
    effects
        .run_reversed_in(&mut tx, EffectPhase::Outbox, r)
        .await?;
    tx.commit().await.map_err(AppError::Database)?;
    Ok(())
}

fn assert_completed_row(row: &OutboxRow, e: &ExchangeCompletedFacts) {
    let tx = e.transaction_id.to_string();
    assert_eq!(row.organization_id, e.organization_id, "{row:?}");
    assert_eq!(row.event_type, "loyalty.exchange_completed", "{row:?}");
    assert_eq!(row.destination, "loyalty", "{row:?}");
    assert_eq!(row.aggregate_type, "transaction", "{row:?}");
    assert_eq!(row.aggregate_id, tx, "{row:?}");
    assert_eq!(row.idempotency_key, format!("exchange:{tx}"), "{row:?}");
    assert_eq!(
        row.payload_redacted,
        serde_json::json!({ "transactionId": tx }),
        "the stored payload is references only (D-4): {row:?}"
    );
}

fn reversal_rows(rows: &[OutboxRow]) -> Vec<&OutboxRow> {
    rows.iter()
        .filter(|row| row.event_type == "loyalty.exchange_reversed")
        .collect()
}

// ---------------------------------------------------------------- completion

#[tokio::test]
async fn a_completed_live_exchange_of_a_verified_member_leaves_one_loyalty_row() {
    let room = Room::start().await;
    let probe = room.sale(None, Mode::Live).await;
    let member = room.member(probe.organization_id).await;
    let e = room.sale(Some(&member), Mode::Live).await;

    complete(&room, &e, Ending::Commit)
        .await
        .expect("the completion commits");

    let rows = room.rows().await;
    assert_eq!(rows.len(), 1, "one loyalty row: {rows:?}");
    assert_completed_row(&rows[0], &e);
    room.finish().await;
}

#[tokio::test]
async fn the_row_is_written_at_the_outbox_step_not_at_redeem() {
    let room = Room::start().await;
    let e0 = room.sale(None, Mode::Live).await;
    let member = room.member(e0.organization_id).await;
    let e = room.sale(Some(&member), Mode::Live).await;
    let effects = room.effects();

    // Redeem alone, committed: the loyalty effect is not a Redeem effect, so nothing is written.
    let mut tx = room.pool().begin().await.expect("begin");
    effects
        .run_completed_in(&mut tx, EffectPhase::Redeem, &e)
        .await
        .expect("redeem step");
    tx.commit().await.expect("commit");
    assert!(
        room.rows().await.is_empty(),
        "nothing at the Redeem step (§6.2: LoyaltyEffect has phase Outbox)"
    );

    // Outbox alone, committed: the row.
    let mut tx = room.pool().begin().await.expect("begin");
    effects
        .run_completed_in(&mut tx, EffectPhase::Outbox, &e)
        .await
        .expect("outbox step");
    tx.commit().await.expect("commit");
    let rows = room.rows().await;
    assert_eq!(rows.len(), 1, "one row at the Outbox step: {rows:?}");
    assert_completed_row(&rows[0], &e);
    room.finish().await;
}

#[tokio::test]
async fn a_completion_that_rolls_back_leaves_no_row() {
    let room = Room::start().await;
    let e0 = room.sale(None, Mode::Live).await;
    let member = room.member(e0.organization_id).await;
    let refused = room.sale(Some(&member), Mode::Live).await;
    let sent = room.sale(Some(&member), Mode::Live).await;

    let err = complete(&room, &refused, Ending::RollBack)
        .await
        .expect_err("a later step refuses");
    assert!(
        err.to_string().contains("stockInsufficient"),
        "the refusal is the answer: {err}"
    );
    complete(&room, &sent, Ending::Commit)
        .await
        .expect("the next completion commits");

    let rows = room.rows().await;
    assert_eq!(
        rows.len(),
        1,
        "the rolled-back completion leaves nothing, the committed one leaves one row: {rows:?}"
    );
    assert_completed_row(&rows[0], &sent);
    room.finish().await;
}

#[tokio::test]
async fn replaying_the_same_completion_leaves_one_row() {
    let room = Room::start().await;
    let e0 = room.sale(None, Mode::Live).await;
    let member = room.member(e0.organization_id).await;
    let e = room.sale(Some(&member), Mode::Live).await;

    complete(&room, &e, Ending::Commit).await.expect("first");
    complete(&room, &e, Ending::Commit)
        .await
        .expect("a replay of the same completion is not an error");

    let rows = room.rows().await;
    assert_eq!(rows.len(), 1, "OB-1's key keeps one row: {rows:?}");
    assert_completed_row(&rows[0], &e);
    room.finish().await;
}

// ---------------------------------------------------------------- D-5: who is sent

#[tokio::test]
async fn a_demo_exchange_leaves_no_row() {
    let room = Room::start().await;
    let e0 = room.sale(None, Mode::Live).await;
    let member = room.member(e0.organization_id).await;
    let demo = room.sale(Some(&member), Mode::Demo).await;
    let live = room.sale(Some(&member), Mode::Live).await;

    complete(&room, &demo, Ending::Commit).await.expect("demo");
    complete(&room, &live, Ending::Commit).await.expect("live");

    let rows = room.rows().await;
    assert_eq!(rows.len(), 1, "only the LIVE exchange is sent: {rows:?}");
    assert_completed_row(&rows[0], &live);
    room.finish().await;
}

#[tokio::test]
async fn an_exchange_with_no_customer_leaves_no_row() {
    let room = Room::start().await;
    let walk_in = room.sale(None, Mode::Live).await;
    let member = room.member(walk_in.organization_id).await;
    let live = room.sale(Some(&member), Mode::Live).await;

    complete(&room, &walk_in, Ending::Commit)
        .await
        .expect("no customer");
    complete(&room, &live, Ending::Commit)
        .await
        .expect("member");

    let rows = room.rows().await;
    assert_eq!(
        rows.len(),
        1,
        "only the member's exchange is sent: {rows:?}"
    );
    assert_completed_row(&rows[0], &live);
    room.finish().await;
}

#[tokio::test]
async fn a_customer_without_an_active_verified_phone_leaves_no_row() {
    let room = Room::start().await;
    let e0 = room.sale(None, Mode::Live).await;
    let org = e0.organization_id;

    // No PHONE identity at all.
    let no_phone = room.customer(org).await;
    // A PHONE identity that was never verified.
    let unverified = room.customer(org).await;
    room.phone(org, &unverified, "ACTIVE", false).await;
    // A verified PHONE that has been REVOKED.
    let revoked = room.customer(org).await;
    room.phone(org, &revoked, "REVOKED", true).await;
    // The control: an ACTIVE verified PHONE.
    let member = room.member(org).await;

    for c in [&no_phone, &unverified, &revoked] {
        let e = room.sale(Some(c), Mode::Live).await;
        complete(&room, &e, Ending::Commit)
            .await
            .expect("the completion still commits; the customer just earns nothing");
    }
    let sent = room.sale(Some(&member), Mode::Live).await;
    complete(&room, &sent, Ending::Commit)
        .await
        .expect("member");

    let rows = room.rows().await;
    assert_eq!(
        rows.len(),
        1,
        "D-5: only an ACTIVE PHONE with verified_at is sent (no phone, unverified and revoked are not): {rows:?}"
    );
    assert_completed_row(&rows[0], &sent);
    room.finish().await;
}

// ---------------------------------------------------------------- reversal

#[tokio::test]
async fn a_posted_reversal_of_a_sent_completion_leaves_one_reversal_row() {
    let room = Room::start().await;
    let e0 = room.sale(None, Mode::Live).await;
    let member = room.member(e0.organization_id).await;
    let e = room.sale(Some(&member), Mode::Live).await;
    complete(&room, &e, Ending::Commit)
        .await
        .expect("completion");
    let r = room.reversal(&e).await;

    post_reversal(&room, &r).await.expect("the reversal posts");

    let rows = room.rows().await;
    assert_eq!(
        rows.len(),
        2,
        "the completion row and one reversal row: {rows:?}"
    );
    let reversals = reversal_rows(&rows);
    assert_eq!(reversals.len(), 1, "{rows:?}");
    let reversal = reversals[0];
    let rev = r.reversal_id.to_string();
    assert_eq!(reversal.organization_id, e.organization_id);
    assert_eq!(reversal.destination, "loyalty");
    assert_eq!(reversal.aggregate_type, "transaction");
    assert_eq!(
        reversal.aggregate_id,
        e.transaction_id.to_string(),
        "the aggregate is the ORIGINAL transaction, so OB-1 never sends the reversal before its earn (D-4)"
    );
    assert_eq!(reversal.idempotency_key, format!("reversal:{rev}"));
    assert_eq!(
        reversal.payload_redacted,
        serde_json::json!({ "reversalId": rev }),
        "references only (D-4)"
    );
    room.finish().await;
}

#[tokio::test]
async fn a_reversal_whose_completion_sent_nothing_leaves_no_reversal_row() {
    let room = Room::start().await;
    let e0 = room.sale(None, Mode::Live).await;
    let org = e0.organization_id;
    // A customer without a verified phone: the completion sends nothing.
    let no_phone = room.customer(org).await;
    let unsent = room.sale(Some(&no_phone), Mode::Live).await;
    complete(&room, &unsent, Ending::Commit)
        .await
        .expect("completion");
    // A DEMO exchange of a member: also sends nothing.
    let member = room.member(org).await;
    let demo = room.sale(Some(&member), Mode::Demo).await;
    complete(&room, &demo, Ending::Commit).await.expect("demo");
    // The control: a sent completion.
    let sent = room.sale(Some(&member), Mode::Live).await;
    complete(&room, &sent, Ending::Commit)
        .await
        .expect("member");

    let r_unsent = room.reversal(&unsent).await;
    let r_demo = room.reversal(&demo).await;
    let r_sent = room.reversal(&sent).await;
    post_reversal(&room, &r_unsent)
        .await
        .expect("reversal of the unsent completion");
    post_reversal(&room, &r_demo)
        .await
        .expect("reversal of the demo completion");
    post_reversal(&room, &r_sent)
        .await
        .expect("reversal of the sent one");

    let rows = room.rows().await;
    let reversals = reversal_rows(&rows);
    assert_eq!(
        reversals.len(),
        1,
        "D-5: a reversal is enqueued only if its completion was: {rows:?}"
    );
    assert_eq!(
        reversals[0].idempotency_key,
        format!("reversal:{}", r_sent.reversal_id)
    );
    assert_eq!(
        rows.len(),
        2,
        "one completion row and one reversal row: {rows:?}"
    );
    room.finish().await;
}

/// Ruling Q2 A: whether the completion was sent is read from its `loyalty.exchange_completed` row, not re-checked
/// from the phone at reversal time. A customer who verifies a phone AFTER the completion gets no reversal row.
#[tokio::test]
async fn a_phone_verified_after_the_completion_does_not_make_its_reversal_sent() {
    let room = Room::start().await;
    let e0 = room.sale(None, Mode::Live).await;
    let org = e0.organization_id;
    let late = room.customer(org).await;
    let unsent = room.sale(Some(&late), Mode::Live).await;
    complete(&room, &unsent, Ending::Commit)
        .await
        .expect("completion before the phone is verified");
    // The customer verifies a phone afterwards.
    room.phone(org, &late, "ACTIVE", true).await;
    // The control: a member's sent completion, reversed too.
    let member = room.member(org).await;
    let sent = room.sale(Some(&member), Mode::Live).await;
    complete(&room, &sent, Ending::Commit)
        .await
        .expect("member");

    let r_late = room.reversal(&unsent).await;
    let r_sent = room.reversal(&sent).await;
    post_reversal(&room, &r_late).await.expect("late reversal");
    post_reversal(&room, &r_sent).await.expect("sent reversal");

    let rows = room.rows().await;
    let reversals = reversal_rows(&rows);
    assert_eq!(
        reversals.len(),
        1,
        "ruling Q2 A: no completion row ⇒ no reversal row, even with a phone verified since: {rows:?}"
    );
    assert_eq!(
        reversals[0].idempotency_key,
        format!("reversal:{}", r_sent.reversal_id)
    );
    room.finish().await;
}

#[tokio::test]
async fn a_reversal_that_rolls_back_leaves_no_reversal_row() {
    let room = Room::start().await;
    let e0 = room.sale(None, Mode::Live).await;
    let member = room.member(e0.organization_id).await;
    let e = room.sale(Some(&member), Mode::Live).await;
    complete(&room, &e, Ending::Commit)
        .await
        .expect("completion");
    let r = room.reversal(&e).await;

    // The reversal's transaction rolls back after the effect step.
    let effects = room.effects();
    let mut tx = room.pool().begin().await.expect("begin");
    effects
        .run_reversed_in(&mut tx, EffectPhase::Outbox, &r)
        .await
        .expect("effect step");
    tx.rollback().await.expect("roll back");
    assert!(
        reversal_rows(&room.rows().await).is_empty(),
        "the effect writes in the caller's transaction, so the rollback leaves no reversal row"
    );

    // The same reversal, committed: one row.
    post_reversal(&room, &r).await.expect("reversal");
    let rows = room.rows().await;
    let reversals = reversal_rows(&rows);
    assert_eq!(reversals.len(), 1, "{rows:?}");
    assert_eq!(
        reversals[0].idempotency_key,
        format!("reversal:{}", r.reversal_id)
    );
    room.finish().await;
}
