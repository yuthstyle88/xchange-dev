//! Written before the implementation (row PM-K, design `docs/design/xchange-v3.md` §5.5.2).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The kiosk honours the channels the branch has switched on, for the direction asked:
//!
//! - `rates` tells the screen the branch's rules (`PublicRates.payment`; the route prints them
//!   as `paymentMethods`, without `account`, in row K-HTTP);
//! - `quote` is refused `Payment method unavailable. Please contact staff.` when the method is
//!   off for the direction — v2's last check, after `Invalid payment method.`
//!   (`server.mjs:7976-7978` of the 10-06 bundle);
//! - `confirm` is refused with the same sentence after the 3-second check and before the rate
//!   check (`server.mjs:7988-7994`); a request already past QUOTED still answers its identity.
//!
//! The rules come from `KioskPorts::payment_rules[_in]`, whose default answers
//! `PaymentRules::defaults()` — so the fakes of the other kiosk tests compile and behave
//! unchanged. `PaymentGate` is pure: what `KioskMachine` answered goes in, and the gate decides
//! whether an off method changes it. `KioskMachine` itself is not edited.
//!
//! The service tests need `TEST_DATABASE_URL` (see `kiosk_service_common/mod.rs`); nothing
//! here skips.

mod kiosk_service_common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use kiosk_service_common::*;
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Side};
use xc_kernel::payment::{ChannelRule, PaymentRules};
use xc_kiosk::domain::kiosk::entities::payment_gate::PaymentGate;
use xc_kiosk::domain::kiosk::value_objects::counter_facts::CounterFacts;
use xc_kiosk::domain::kiosk::value_objects::kiosk_event_kind::KioskEventKind;
use xc_kiosk::domain::kiosk::value_objects::transition::Transition;

const UNAVAILABLE: &str = "Payment method unavailable. Please contact staff.";
const REVIEW_3S: &str = "Please review your quote for 3 seconds.";
const RATE_CHANGED: &str = "Rate changed or counter closed. Please start again.";
const QUOTE_EXPIRED: &str = "Quote expired. Please check the rate again.";
const NOT_FOUND: &str = "Request not found. Please start again.";
const BUY_CASH_OR_TRANSFER: &str = "Select cash or bank transfer to receive THB.";
const INVALID_METHOD: &str = "Invalid payment method.";
const COUNTER_CLOSED: &str = "Counter is closed. Please contact our staff.";

/// The defaults with one channel switched off.
fn off(id: &str) -> PaymentRules {
    change(id, |r| r.enabled = false)
}

fn change(id: &str, f: impl Fn(&mut ChannelRule)) -> PaymentRules {
    let rows = PaymentRules::defaults()
        .rows()
        .iter()
        .cloned()
        .map(|mut r| {
            if r.id == id {
                f(&mut r);
            }
            r
        })
        .collect();
    PaymentRules::from_rows(rows).expect("rules v2 accepts")
}

fn quote_with(id: &str, side: Side, method: Option<&str>) -> NewQuote {
    let mut quote = quote_of(id, side, "USD", "100");
    quote.input.payment_method = method.map(str::to_owned);
    quote
}

/// A request as the machine makes it, at `T0`.
fn request(side: Side, method: &str) -> KioskRequest {
    let input = QuoteInput {
        side: Some(side),
        currency: CurrencyCode::parse("USD").ok(),
        amount: Money::parse("100").ok(),
        payment_method: Some(method.to_owned()),
    };
    let facts = CounterFacts {
        open: true,
        rate: Some(rate("32.1000")),
    };
    KioskMachine::quote(&input, &facts, "q1", KioskTime::from_epoch_millis(T0))
        .unwrap_or_else(|e| panic!("the machine quotes {side:?} {method}: {e}"))
}

fn waiting(request: &KioskRequest) -> Transition {
    let mut moved = request.clone();
    moved.status = KioskStatus::Waiting;
    Transition {
        request: moved,
        event: Some(KioskEventKind::Request),
    }
}

// ── the gate, alone ──────────────────────────────────────────────────────────

#[test]
fn the_gate_lets_a_quote_through_only_when_its_method_is_on_for_its_direction() {
    let sell_card = request(Side::Sell, "Credit Card");
    assert!(PaymentGate::quote(&sell_card, &PaymentRules::defaults()).is_ok());
    assert_eq!(
        PaymentGate::quote(&sell_card, &off("Credit Card"))
            .expect_err("Credit Card is off")
            .to_string(),
        UNAVAILABLE
    );
    // on, but not for SELL (only Cash and Bank Transfer can be on for BUY alone)
    let transfer_buy_only = change("Bank Transfer", |r| r.sell = false);
    assert_eq!(
        PaymentGate::quote(&request(Side::Sell, "Bank Transfer"), &transfer_buy_only)
            .expect_err("Bank Transfer takes no baht in")
            .to_string(),
        UNAVAILABLE
    );
    assert!(PaymentGate::quote(&request(Side::Buy, "Bank Transfer"), &transfer_buy_only).is_ok());
    // the direction is the request's: Cash off for SELL only still pays baht out
    let cash_buy_only = change("Cash", |r| r.sell = false);
    assert!(PaymentGate::quote(&request(Side::Buy, "Cash"), &cash_buy_only).is_ok());
    assert_eq!(
        PaymentGate::quote(&request(Side::Sell, "Cash"), &cash_buy_only)
            .expect_err("Cash takes no baht in")
            .to_string(),
        UNAVAILABLE
    );
}

#[test]
fn confirm_moving_to_waiting_is_refused_when_the_method_was_switched_off() {
    let stored = request(Side::Sell, "PromptPay");
    let moved = waiting(&stored);
    assert_eq!(
        PaymentGate::confirm(Ok(moved.clone()), Some(&stored), &PaymentRules::defaults()),
        Ok(moved.clone())
    );
    let refused = PaymentGate::confirm(Ok(moved), Some(&stored), &off("PromptPay"))
        .expect_err("PromptPay is off");
    assert_eq!(refused.to_string(), UNAVAILABLE);
}

/// v2 checks the method before the rate: an off method wins over a changed rate or a closed
/// counter.
#[test]
fn confirm_an_off_method_wins_over_a_changed_rate() {
    let stored = request(Side::Sell, "Alipay");
    let changed = Err(KioskError::RateChangedOrClosed);
    assert_eq!(
        PaymentGate::confirm(changed.clone(), Some(&stored), &off("Alipay"))
            .expect_err("refused")
            .to_string(),
        UNAVAILABLE
    );
    assert_eq!(
        PaymentGate::confirm(changed, Some(&stored), &PaymentRules::defaults())
            .expect_err("refused")
            .to_string(),
        RATE_CHANGED
    );
}

/// What the machine refuses before the payment check stays the machine's answer.
#[test]
fn confirm_the_machines_earlier_refusals_come_first() {
    let stored = request(Side::Sell, "Alipay");
    for (machine, sentence) in [
        (KioskError::ReviewThreeSeconds, REVIEW_3S),
        (KioskError::QuoteExpired, QUOTE_EXPIRED),
    ] {
        assert_eq!(
            PaymentGate::confirm(Err(machine), Some(&stored), &off("Alipay"))
                .expect_err("refused")
                .to_string(),
            sentence
        );
    }
    assert_eq!(
        PaymentGate::confirm(
            Err(KioskError::RequestNotFoundStartAgain),
            None,
            &off("Alipay")
        )
        .expect_err("refused")
        .to_string(),
        NOT_FOUND
    );
}

/// A request already past QUOTED answers its identity, whatever was switched off since.
#[test]
fn confirm_a_request_past_quoted_answers_its_identity_even_if_its_method_is_off() {
    let mut stored = request(Side::Sell, "Alipay");
    for status in [
        KioskStatus::Waiting,
        KioskStatus::Accepted,
        KioskStatus::IdChecked,
        KioskStatus::Completed,
    ] {
        stored.status = status;
        let unchanged = Transition {
            request: stored.clone(),
            event: None,
        };
        assert_eq!(
            PaymentGate::confirm(Ok(unchanged.clone()), Some(&stored), &off("Alipay")),
            Ok(unchanged),
            "{status:?}"
        );
    }
}

// ── the service, with the branch's rules in the test's hands ─────────────────

/// `FakePorts` with each branch's payment rules set by the test (defaults otherwise).
struct Switched {
    inner: Arc<FakePorts>,
    rules: Mutex<BTreeMap<String, PaymentRules>>,
}

impl Switched {
    fn new() -> Arc<Switched> {
        Arc::new(Switched {
            inner: FakePorts::standard(),
            rules: Mutex::new(BTreeMap::new()),
        })
    }

    fn set(&self, branch: &str, rules: PaymentRules) {
        self.rules
            .lock()
            .expect("the lock")
            .insert(branch.to_owned(), rules);
    }

    fn of(&self, branch: &BranchId) -> PaymentRules {
        self.rules
            .lock()
            .expect("the lock")
            .get(branch.as_str())
            .cloned()
            .unwrap_or_else(PaymentRules::defaults)
    }
}

impl KioskPorts for Switched {
    fn branch_name<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Option<String>>> {
        self.inner.branch_name(branch)
    }

    fn counter<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<CounterView>>> {
        self.inner.counter(branch)
    }

    fn counter_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<CounterView>>> {
        self.inner.counter_in(tx, branch)
    }

    fn save_rate<'a>(
        &'a self,
        branch: &'a BranchId,
        rate: &'a SaveCounterRate,
        actor: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.inner.save_rate(branch, rate, actor, now)
    }

    fn record_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line: &'a AuditLine,
        at: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.inner.record_in(tx, branch, line, at)
    }

    fn payment_rules<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<PaymentRules>> {
        let rules = self.of(branch);
        Box::pin(async move { Ok(rules) })
    }

    fn payment_rules_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<PaymentRules>> {
        let rules = self.of(branch);
        Box::pin(async move { Ok(rules) })
    }
}

fn service(db: &TestDb, ports: &Arc<Switched>) -> KioskService {
    KioskService::new(db.branch_tx(), ports.clone())
}

/// The ports trait answers the defaults unless a branch's rules say otherwise: the fakes of
/// every other kiosk test, which do not name the two methods, are held to v2's defaults.
#[tokio::test]
async fn the_ports_default_is_v2s_defaults() {
    let ports = FakePorts::standard();
    let defaults = ports
        .payment_rules(&branch(MAIN))
        .await
        .expect("the default answer");
    assert_eq!(defaults, PaymentRules::defaults());
}

#[tokio::test]
async fn rates_carry_the_branchs_own_rules() {
    let db = TestDb::start().await;
    let ports = Switched::new();
    ports.set(MAIN, off("Credit Card"));
    let service = service(&db, &ports);
    let main = service.public_rates(&branch(MAIN)).await.expect("MAIN");
    assert_eq!(main.payment, off("Credit Card"));
    let b2 = service.public_rates(&branch(B2)).await.expect("B2");
    assert_eq!(b2.payment, PaymentRules::defaults());
    db.finish().await;
}

#[tokio::test]
async fn a_quote_on_a_method_that_is_off_is_refused_and_nothing_is_stored() {
    let db = TestDb::start().await;
    let ports = Switched::new();
    ports.set(MAIN, off("Credit Card"));
    let service = service(&db, &ports);
    assert_eq!(
        refusal(
            service
                .quote(
                    &branch(MAIN),
                    quote_with("q1", Side::Sell, Some("Credit Card")),
                    at(0)
                )
                .await
        ),
        UNAVAILABLE
    );
    assert_eq!(db.count("kiosk_requests").await, 0);
    // another branch keeps its own switch
    service
        .quote(
            &branch(B2),
            quote_with("q2", Side::Sell, Some("Credit Card")),
            at(1),
        )
        .await
        .expect("B2 takes cards");
    // and a channel that is on in MAIN is quoted there
    service
        .quote(
            &branch(MAIN),
            quote_with("q3", Side::Sell, Some("PromptPay")),
            at(2),
        )
        .await
        .expect("PromptPay is on");
    assert_eq!(db.count("kiosk_requests").await, 2);
    db.finish().await;
}

/// No method is Cash (`server.mjs:7975`): Cash off for BUY refuses a quote that names none.
#[tokio::test]
async fn a_quote_with_no_method_is_cash() {
    let db = TestDb::start().await;
    let ports = Switched::new();
    ports.set(MAIN, change("Cash", |r| r.buy = false));
    let service = service(&db, &ports);
    for (id, method) in [("q1", None), ("q2", Some("")), ("q3", Some("Cash"))] {
        assert_eq!(
            refusal(
                service
                    .quote(&branch(MAIN), quote_with(id, Side::Buy, method), at(0))
                    .await
            ),
            UNAVAILABLE,
            "{method:?}"
        );
    }
    service
        .quote(
            &branch(MAIN),
            quote_with("q4", Side::Buy, Some("Bank Transfer")),
            at(1),
        )
        .await
        .expect("transfer still pays baht out");
    db.finish().await;
}

/// The payment check is the last: what the machine refuses first is still its sentence.
#[tokio::test]
async fn the_kiosks_own_quote_refusals_come_first() {
    let db = TestDb::start().await;
    let ports = Switched::new();
    let mut nothing_but_transfer = PaymentRules::defaults().rows().to_vec();
    for r in &mut nothing_but_transfer {
        r.enabled = r.id == "Bank Transfer";
    }
    ports.set(
        MAIN,
        PaymentRules::from_rows(nothing_but_transfer).expect("transfer only"),
    );
    let service = service(&db, &ports);
    let main = branch(MAIN);
    assert_eq!(
        refusal(
            service
                .quote(
                    &main,
                    quote_with("q1", Side::Buy, Some("Credit Card")),
                    at(0)
                )
                .await
        ),
        BUY_CASH_OR_TRANSFER
    );
    assert_eq!(
        refusal(
            service
                .quote(&main, quote_with("q2", Side::Sell, Some("Cheque")), at(0))
                .await
        ),
        INVALID_METHOD
    );
    ports.inner.set_open(MAIN, false);
    assert_eq!(
        refusal(
            service
                .quote(&main, quote_with("q3", Side::Sell, Some("Cash")), at(0))
                .await
        ),
        COUNTER_CLOSED
    );
    db.finish().await;
}

/// "คิวค้างที่ใช้ช่องทางปิดต้องให้ลูกค้าเลือกใหม่": a quote made while the channel was on is
/// refused at confirm once it is off, and stays QUOTED; switched on again, it confirms.
#[tokio::test]
async fn a_quote_confirmed_after_its_method_was_switched_off_is_refused() {
    let db = TestDb::start().await;
    let ports = Switched::new();
    let service = service(&db, &ports);
    let main = branch(MAIN);
    service
        .quote(&main, quote_with("q1", Side::Sell, Some("Alipay")), at(0))
        .await
        .expect("Alipay is on");

    ports.set(MAIN, off("Alipay"));
    assert_eq!(
        refusal(service.confirm(&main, "q1", at(1_000)).await),
        REVIEW_3S,
        "the 3-second check comes first"
    );
    assert_eq!(
        refusal(service.confirm(&main, "q1", at(4_000)).await),
        UNAVAILABLE
    );
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("QUOTED"));

    // the rate changes too: the method still wins
    ports
        .inner
        .set_rate(MAIN, counter_rate("USD", "32.0000", "32.5000"));
    assert_eq!(
        refusal(service.confirm(&main, "q1", at(5_000)).await),
        UNAVAILABLE
    );
    ports
        .inner
        .set_rate(MAIN, counter_rate("USD", "32.1000", "32.6000"));

    ports.set(MAIN, PaymentRules::defaults());
    service
        .confirm(&main, "q1", at(6_000))
        .await
        .expect("on again");
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("WAITING"));
    db.finish().await;
}

/// A request already in the queue is not taken back: confirm answers its identity.
#[tokio::test]
async fn a_request_already_waiting_answers_its_identity_after_the_switch() {
    let db = TestDb::start().await;
    let ports = Switched::new();
    let service = service(&db, &ports);
    let main = branch(MAIN);
    service
        .quote(
            &main,
            quote_with("q1", Side::Sell, Some("PromptPay")),
            at(0),
        )
        .await
        .expect("quote");
    let first = service
        .confirm(&main, "q1", at(4_000))
        .await
        .expect("confirm");
    ports.set(MAIN, off("PromptPay"));
    let again = service
        .confirm(&main, "q1", at(5_000))
        .await
        .expect("an identity, not a refusal");
    assert_eq!(again, first);
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("WAITING"));
    db.finish().await;
}
