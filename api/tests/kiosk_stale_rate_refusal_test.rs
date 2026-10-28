//! Written before the implementation by the sub-controller claude/studio-claude-16 (controller
//! claude/553782). Do not edit, delete or weaken.
//!
//! Ticket #5418: a kiosk quote on a stale line (policy `STOP`) must answer the rate refusal,
//! `KioskError::RateChangedOrClosed` ("Rate changed or counter closed. Please start again."),
//! never `AmountOrCurrencyInvalid`, which tells the customer their own input is wrong. The tier
//! refusal (`customer.rs`, the repricing loop that does not settle in 4 rounds) keeps
//! `AmountOrCurrencyInvalid`.
//!
//! What this pins (#5418 §3, #5432):
//! - control: a line the stale check admits is quoted, so the scene reaches the stale check;
//! - K1: `check_stale_in` answers `false` ⇒ `RateChangedOrClosed`, and no request is written;
//! - K2: `check_stale_in` admits, but `quote_in` answers `rateUnavailable` ⇒ `RateChangedOrClosed`;
//! - K3: `tier_line_in` never settles ⇒ `AmountOrCurrencyInvalid`, unchanged, and the stale
//!   check is not asked;
//! - K3 control: a non-stale `Domain` refusal from `quote_in` still answers `AmountOrCurrencyInvalid`.
//!
//! The kiosk service is real and so is its repository; the other contexts (`KioskPorts`) are
//! answered by `Counter` below, which can be told the line is stale or that tiers oscillate.
//! Since ruling B4 (#5168) the real `tier_line_in` answers tier 1 for every amount, so only a
//! fake reaches the loop's refusal; `rate_feed_kiosk_tier_test.rs` pins the real ports' side.
//!
//! The refusal sentences are read from `KioskError` itself, never copied. Needs a real
//! PostgreSQL (`TEST_DATABASE_URL`), as `kiosk_save_step_test.rs` does.

#[allow(unused_imports)]
mod save_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use save_common::*;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, MinorUnit, Money, Rate, Side, UnitSize};
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::commands::new_quote::NewQuote;
use xc_kiosk::domain::kiosk::commands::quote_input::QuoteInput;
use xc_kiosk::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use xc_kiosk::domain::kiosk::value_objects::audit_line::AuditLine;
use xc_kiosk::domain::kiosk::value_objects::counter_facts::{CounterLine, LineId};
use xc_kiosk::domain::kiosk::value_objects::counter_view::{CounterRate, CounterView};
use xc_kiosk::domain::kiosk::value_objects::kiosk_error::KioskError;

type Asked<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

/// The counter's one published line, and the other tier line the oscillating fake answers.
const LINE: LineId = LineId(1);
const OTHER_TIER: LineId = LineId(2);

/// What the kiosk asks of the other contexts: an open MAIN buying USD at 32.1 on `LINE`.
#[derive(Default)]
struct Counter {
    /// K1: the stale check refuses (a stale line under policy `STOP`).
    stale: AtomicBool,
    /// K3: `tier_line_in` answers the other tier line each round, so the loop never settles.
    oscillates: AtomicBool,
    /// K2 / K3 control: `quote_in` answers this Domain error (e.g. `rateUnavailable: ...`).
    quote_refusal: Mutex<Option<String>>,
    /// How many times `tier_line_in` was asked.
    tier_asks: AtomicUsize,
    /// How many times `check_stale_in` was asked.
    stale_asks: AtomicUsize,
    /// How many times `quote_in` was asked.
    quote_asks: AtomicUsize,
}

impl Counter {
    fn view() -> CounterView {
        CounterView {
            branch_name: "Main".to_owned(),
            open: true,
            rates: vec![CounterRate {
                code: code("USD"),
                buy: Rate::parse("32.1").expect("a rate"),
                sell: Rate::parse("33").expect("a rate"),
                unit_size: UnitSize::ONE,
                minor_unit: MinorUnit::TWO,
                line_id: LINE,
            }],
        }
    }

    fn tier_line(line_id: LineId) -> CounterLine {
        CounterLine {
            line_id,
            rate: Rate::parse("32.1").expect("a rate"),
            unit_size: UnitSize::ONE,
            minor_unit: MinorUnit::TWO,
        }
    }
}

impl KioskPorts for Counter {
    fn branch_name<'a>(&'a self, _branch: &'a BranchId) -> Asked<'a, Option<String>> {
        Box::pin(async { Ok(Some("Main".to_owned())) })
    }

    fn counter<'a>(&'a self, _branch: &'a BranchId) -> Asked<'a, Option<CounterView>> {
        Box::pin(async { Ok(Some(Self::view())) })
    }

    fn counter_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> Asked<'a, Option<CounterView>> {
        Box::pin(async move {
            the_line_exists(tx, branch).await?;
            Ok(Some(Self::view()))
        })
    }

    fn record_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _line: &'a AuditLine,
        _at: DateTime<Utc>,
    ) -> Asked<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    #[allow(clippy::too_many_arguments)]
    fn check_stale_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _line_id: LineId,
        _currency: CurrencyCode,
        _side: Side,
        _thb: Money,
        _now: DateTime<Utc>,
    ) -> Asked<'a, bool> {
        self.stale_asks.fetch_add(1, Ordering::SeqCst);
        let admitted = !self.stale.load(Ordering::SeqCst);
        Box::pin(async move { Ok(admitted) })
    }

    fn tier_line_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _currency: CurrencyCode,
        _side: Side,
        _foreign: ForeignAmount,
    ) -> Asked<'a, Option<CounterLine>> {
        let round = self.tier_asks.fetch_add(1, Ordering::SeqCst);
        let answer = if self.oscillates.load(Ordering::SeqCst) {
            // Round 0 moves the loop off `LINE`, round 1 back onto it, and so on.
            Some(Self::tier_line(if round.is_multiple_of(2) {
                OTHER_TIER
            } else {
                LINE
            }))
        } else {
            None
        };
        Box::pin(async move { Ok(answer) })
    }

    fn quote_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _currency: CurrencyCode,
        _side: Side,
        _quantity: ForeignAmount,
        _now: DateTime<Utc>,
    ) -> Asked<'a, Option<String>> {
        self.quote_asks.fetch_add(1, Ordering::SeqCst);
        let error = self.quote_refusal.lock().unwrap().clone();
        Box::pin(async move {
            match error {
                Some(msg) => Err(AppError::Domain(msg)),
                None => Ok(None),
            }
        })
    }
}

struct Scene {
    shop: Shop,
    counter: Arc<Counter>,
    kiosk: KioskService,
}

impl Scene {
    async fn start() -> Self {
        let shop = Shop::start().await;
        let counter = Arc::new(Counter::default());
        let branch_tx = BranchTx::new(Transactor::new(shop.pool.clone()));
        let kiosk = KioskService::new(branch_tx, counter.clone());
        Self {
            shop,
            counter,
            kiosk,
        }
    }

    /// The customer's request at the kiosk: buy 100 USD, paid in cash (#5418 §1's `BUY USD 100`).
    async fn quote(&self, id: &str) -> AppResult<()> {
        let quote = NewQuote {
            input: QuoteInput {
                side: Some(Side::Buy),
                currency: Some(code("USD")),
                amount: Some(money("100")),
                payment_method: Some("Cash".to_owned()),
            },
            new_id: id.to_owned(),
            client_ip: "203.0.113.7".to_owned(),
        };
        self.kiosk
            .quote(&branch(MAIN), quote, at(0))
            .await
            .map(|_| ())
    }

    async fn requests(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM kiosk_requests")
            .fetch_one(&self.shop.pool)
            .await
            .expect("count the requests")
    }
}

/// The sentence a refusal carries; anything but a refusal fails the test.
fn refusal(result: AppResult<()>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected a refusal (AppError::Domain), got {other:?}"),
    }
}

#[tokio::test]
async fn a_line_the_stale_check_admits_is_quoted() {
    let scene = Scene::start().await;

    scene
        .quote("00000000-0000-4000-8000-000000541800")
        .await
        .expect("a fresh line is quoted");

    assert_eq!(
        scene.counter.stale_asks.load(Ordering::SeqCst),
        1,
        "the quote reached the stale check once"
    );
    assert_eq!(scene.requests().await, 1, "the quote wrote its request");
}

#[tokio::test]
async fn k1_a_stale_line_is_refused_as_a_rate_change_not_as_bad_input() {
    let scene = Scene::start().await;
    scene.counter.stale.store(true, Ordering::SeqCst);

    let sentence = refusal(scene.quote("00000000-0000-4000-8000-000000541801").await);

    assert_eq!(
        sentence,
        KioskError::RateChangedOrClosed.to_string(),
        "#5418 K1: a stale line (on_stale STOP) is the rate's fault, not the customer's input; \
         `{}` tells them their amount or currency is wrong",
        KioskError::AmountOrCurrencyInvalid
    );
    assert_eq!(
        scene.counter.stale_asks.load(Ordering::SeqCst),
        1,
        "the refusal came from the stale check"
    );
    assert_eq!(
        scene.requests().await,
        0,
        "a refused quote writes no request"
    );
}

#[tokio::test]
async fn k2_stale_refusal_from_quote_in_answers_rate_changed_or_closed() {
    let scene = Scene::start().await;
    *scene.counter.quote_refusal.lock().unwrap() =
        Some("rateUnavailable: stale line refused".to_owned());

    let sentence = refusal(scene.quote("00000000-0000-4000-8000-000000541802").await);

    assert_eq!(
        sentence,
        KioskError::RateChangedOrClosed.to_string(),
        "#5432 K2: quote_in returning rateUnavailable is a rate change, not bad customer input"
    );
    assert_eq!(
        scene.counter.stale_asks.load(Ordering::SeqCst),
        1,
        "the quote passed check_stale_in before quote_in"
    );
    assert_eq!(
        scene.counter.quote_asks.load(Ordering::SeqCst),
        1,
        "the quote reached quote_in"
    );
    assert_eq!(
        scene.requests().await,
        0,
        "a refused quote writes no request"
    );
}

#[tokio::test]
async fn k3_tiers_that_never_settle_are_still_refused_as_an_invalid_amount() {
    let scene = Scene::start().await;
    scene.counter.oscillates.store(true, Ordering::SeqCst);

    let sentence = refusal(scene.quote("00000000-0000-4000-8000-000000541803").await);

    assert_eq!(
        sentence,
        KioskError::AmountOrCurrencyInvalid.to_string(),
        "#5418 K3: the repricing loop's refusal is unchanged"
    );
    assert_eq!(
        scene.counter.tier_asks.load(Ordering::SeqCst),
        4,
        "the loop asked for the tier line in each of its 4 rounds"
    );
    assert_eq!(
        scene.counter.stale_asks.load(Ordering::SeqCst),
        0,
        "the loop refused before the stale check"
    );
    assert_eq!(
        scene.requests().await,
        0,
        "a refused quote writes no request"
    );
}

#[tokio::test]
async fn k3_control_non_stale_domain_refusal_from_quote_in_answers_invalid_amount() {
    let scene = Scene::start().await;
    *scene.counter.quote_refusal.lock().unwrap() =
        Some("validationFailed: amount out of range".to_owned());

    let sentence = refusal(scene.quote("00000000-0000-4000-8000-000000541804").await);

    assert_eq!(
        sentence,
        KioskError::AmountOrCurrencyInvalid.to_string(),
        "#5432 K3 control: non-stale domain refusal from quote_in still answers AmountOrCurrencyInvalid"
    );
    assert_eq!(
        scene.counter.stale_asks.load(Ordering::SeqCst),
        1,
        "the quote passed check_stale_in before quote_in"
    );
    assert_eq!(
        scene.counter.quote_asks.load(Ordering::SeqCst),
        1,
        "the quote reached quote_in"
    );
    assert_eq!(
        scene.requests().await,
        0,
        "a refused quote writes no request"
    );
}

/// A stored quote references its line, so the counter's one line is a row of
/// `published_rate_lines` (with its approved request and publication), written in the caller's
/// transaction the first time the counter is read (as `kiosk_save_step_test.rs` does).
async fn the_line_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
) -> AppResult<()> {
    let line = LINE.to_string();
    let request = format!("{line}-{}", branch.as_str());
    sqlx::query(
        "WITH r AS ( \
           INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
               object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
               status, idempotency_key) \
           SELECT gen_random_uuid(), organization_id, 'RATE_PUBLICATION', id, $1::uuid, 'test', \
               '1', '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
           FROM branches WHERE id = $2 \
             AND NOT EXISTS (SELECT 1 FROM published_rate_lines WHERE id = $1::uuid) \
           RETURNING id, organization_id, branch_id), \
         p AS ( \
           INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
               approved_request_id, draft_id, published_by) \
           SELECT $1::uuid, r.organization_id, r.branch_id, \
               COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = r.branch_id), 0) + 1, \
               now(), r.id, NULL, 'test' FROM r \
           RETURNING id, organization_id) \
         INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
         SELECT p.id, p.organization_id, p.id, 'USD', 'ALL', 1, 32.1, 33, \
             '{\"buy_mode\": \"MANUAL\", \"sell_mode\": \"MANUAL\"}'::jsonb FROM p",
    )
    .bind(&line)
    .bind(branch.as_str())
    .bind(&request)
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    Ok(())
}
