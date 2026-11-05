//! Up-front test for CS-20 (design `docs/design/configurable-shifts.md` §2.10, T9): the completion's
//! `ExchangeCompletedFacts.reversible_until` (`complete.rs` `branch_day_in`) is the end of the sale's business day
//! under the plan, not local midnight after `value_date`. Written by the ticket author claude/35271802 — the
//! implementer must not edit, delete or weaken it.
//!
//! The quote and the completion run at today's 12:00 Bangkok, as `rv3_reversal_post_test.rs` does, so the published
//! rates are valid. MAIN keeps the defaults (cutover 06:00), so the day ends tomorrow 06:00 Bangkok. Needs
//! `TEST_DATABASE_URL`.

mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
use chrono_tz::Asia::Bangkok;
use pos_error::AppResult;
use tx4_common::*;
use tx5_common::*;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::complete::{
    BoxFuture, CompleteExchange, EffectPhase, ExchangeCompletedFacts, ExchangeEffect,
    ExchangeEffects, PostedReversal,
};

/// Records the facts every completion hands its effects.
struct Facts {
    seen: Arc<Mutex<Vec<ExchangeCompletedFacts>>>,
}

impl ExchangeEffect for Facts {
    fn name(&self) -> &'static str {
        "cs20-facts"
    }

    fn phase(&self) -> EffectPhase {
        EffectPhase::Consume
    }

    fn on_completed_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        e: &'a ExchangeCompletedFacts,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.seen.lock().unwrap().push(e.clone());
        Box::pin(async { Ok(()) })
    }

    fn on_reversed_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _r: &'a PostedReversal,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

fn today() -> NaiveDate {
    Utc::now().with_timezone(&Bangkok).date_naive()
}

fn bangkok(d: NaiveDate, h: u32) -> DateTime<Utc> {
    Bangkok
        .from_local_datetime(&d.and_hms_opt(h, 0, 0).unwrap())
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

/// C1: a sale dated today, completed at 12:00, may be reversed until tomorrow 06:00 Bangkok, not tomorrow 00:00.
#[tokio::test]
async fn c1_the_completion_hands_effects_the_end_of_the_business_day() {
    let c = Counter::start().await;
    let noon = bangkok(today(), 12);
    let q = c.desk.quote("MAIN", Side::Buy, "100", None, noon).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, noon).await;
    c.desk
        .db
        .exec(&format!(
            "UPDATE transactions SET value_date = '{}' WHERE id = '{}'",
            today(),
            t.id.0
        ))
        .await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    let effect: Arc<dyn ExchangeEffect> = Arc::new(Facts { seen: seen.clone() });
    let service = CompleteExchange::new(
        c.desk.db.pool.clone(),
        c.files.clone(),
        Arc::new(LedgerCashPostings::new()),
        Vec::new(),
        Arc::new(ExchangeEffects::new(vec![effect])),
    );
    let mut tx = c.desk.db.pool.begin().await.unwrap();
    service
        .run_in(&mut tx, t.id, &teller(), later(noon, 5))
        .await
        .unwrap_or_else(|e| panic!("complete: {e:?}"));
    tx.rollback().await.unwrap();

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "one completion");
    let tomorrow = today().checked_add_days(Days::new(1)).unwrap();
    assert_eq!(seen[0].business_date, today());
    assert_eq!(
        seen[0].reversible_until,
        bangkok(tomorrow, 6),
        "the end of today's business day is tomorrow 06:00 Bangkok, not midnight"
    );
    c.finish().await;
}
