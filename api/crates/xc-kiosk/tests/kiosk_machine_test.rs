//! The kiosk machine: what a customer is quoted, how long the quote lasts, and who at
//! the desk may take the request how far.
//!
//! Provided with ticket T12a. These tests are the specification: make them pass,
//! do not edit, delete or weaken them.
//!
//! The first part reads as the rules. The second part holds the machine to every answer
//! the v2 server gave (`fixtures/v2_kiosk_cases.json`, written by
//! `api/tools/v2-fixtures/gen_kiosk.mjs`).

mod kiosk_common;

use kiosk_common::*;
use xc_kernel::money::Side;

use KioskCommand as Cmd;
use KioskError as E;
use KioskStatus as S;

/// Noon in Bangkok on 6 October 2026.
const T0: i64 = 1_791_262_800_000;
const SEC: i64 = 1_000;
const MIN: i64 = 60_000;
const ID: &str = "3fa9c1d2-7e57-4000-8000-00000000beef";

fn open(rate_text: &str) -> CounterFacts {
    CounterFacts {
        open: true,
        rate: Some(rate(rate_text)),
    }
}

fn wants(side: Side, currency: &str, amount: &str) -> QuoteInput {
    QuoteInput {
        side: Some(side),
        currency: Some(code(currency)),
        amount: Some(money(amount)),
        payment_method: None,
    }
}

fn paying(input: QuoteInput, method: &str) -> QuoteInput {
    QuoteInput {
        payment_method: Some(method.into()),
        ..input
    }
}

fn quote_at(input: &QuoteInput, facts: &CounterFacts) -> Result<KioskRequest, KioskError> {
    KioskMachine::quote(input, facts, ID, at(T0))
}

/// 100 dollars sold by the customer at 32.10, quoted at `T0`.
fn quoted() -> KioskRequest {
    quote_at(&wants(Side::Buy, "USD", "100"), &open("32.1")).expect("a plain quote")
}

fn customer() -> KioskActor {
    KioskActor::Customer
}

fn front1() -> KioskActor {
    staff("front1", KioskRole::Front)
}

fn front2() -> KioskActor {
    staff("front2", KioskRole::Front)
}

fn step(
    request: &KioskRequest,
    command: Cmd,
    who: &KioskActor,
    facts: &CounterFacts,
    now: i64,
) -> Result<Transition, KioskError> {
    KioskMachine::next(Some(request), &command, who, facts, at(now))
}

fn confirm(request: &KioskRequest, now: i64) -> Result<Transition, KioskError> {
    step(request, Cmd::Confirm, &customer(), &open("32.1"), now)
}

fn checked(confirmed: bool) -> Cmd {
    Cmd::IdentityChecked { confirmed }
}

fn waiting() -> KioskRequest {
    confirm(&quoted(), T0 + 10 * SEC).expect("confirm").request
}

fn accepted() -> KioskRequest {
    step(
        &waiting(),
        Cmd::Accept,
        &front1(),
        &no_facts(),
        T0 + 20 * SEC,
    )
    .expect("accept")
    .request
}

fn id_checked() -> KioskRequest {
    step(
        &accepted(),
        checked(true),
        &front1(),
        &no_facts(),
        T0 + 30 * SEC,
    )
    .expect("passport check")
    .request
}

fn completed() -> KioskRequest {
    KioskMachine::complete(&id_checked(), "SUB-1", "XC-20261006-000001")
        .expect("complete")
        .request
}

fn matching_save() -> SaveAttempt {
    SaveAttempt {
        live: true,
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
        method: "Cash".into(),
        submission_id: "SUB-1".into(),
    }
}

fn refusal<T: std::fmt::Debug>(got: Result<T, KioskError>) -> KioskError {
    got.expect_err("expected a refusal")
}

// ═════════════════════════════════════════════════════════════════════════════
// The quote
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn a_quote_buys_the_amount_named_and_records_what_it_was_given() {
    let made = quote_at(&wants(Side::Buy, "USD", "250.50"), &open("32.1")).unwrap();
    assert_eq!(
        made,
        KioskRequest {
            id: ID.into(),
            status: S::Quoted,
            created: at(T0),
            staff: None,
            quote: KioskQuote {
                side: Side::Buy,
                currency: code("USD"),
                rate: rate("32.1"),
                foreign: money("250.50"),
                thb: money("8041.05"),
                input_amount: money("250.50"),
                payment_method: PaymentMethod::Cash,
            },
            transaction: None,
        }
    );
}

#[test]
fn selling_hands_over_what_the_baht_named_can_pay_for_rounded_down() {
    // 1,000 baht at 32.60 is 30.6748… dollars: the customer gets 30.67 and pays for exactly that
    let q = quote_at(&wants(Side::Sell, "USD", "1000"), &open("32.6"))
        .unwrap()
        .quote;
    assert_eq!(
        (q.foreign, q.thb, q.input_amount),
        (money("30.67"), money("999.84"), money("1000"))
    );
    // never more baht than were named
    for baht in ["0.33", "1", "99.99", "5000", "9999999.99", "10000000"] {
        for r in ["32.6", "38.2", "0.2195", "24.6", "0.0001", "100000000"] {
            if let Ok(made) = quote_at(&wants(Side::Sell, "USD", baht), &open(r)) {
                assert!(made.quote.thb <= money(baht), "{baht} at {r}");
                assert!(!made.quote.foreign.is_zero(), "{baht} at {r}");
            }
        }
    }
}

#[test]
fn figures_are_exact_where_v2_computed_in_binary_floating_point() {
    // 14.67 baht buy exactly 0.45 dollars at 32.60; v2's division landed below and it gave 0.44
    let q = quote_at(&wants(Side::Sell, "USD", "14.67"), &open("32.6"))
        .unwrap()
        .quote;
    assert_eq!((q.foreign, q.thb), (money("0.45"), money("14.67")));
    // 10 euros at 37.5055 are 375.055 baht: half a hundredth rounds up; v2 answered 375.05
    let q = quote_at(&wants(Side::Buy, "EUR", "10"), &open("37.5055"))
        .unwrap()
        .quote;
    assert_eq!(q.thb, money("375.06"));
    // a figure the machine computes may pass the cap on a figure a person enters
    let q = quote_at(&wants(Side::Buy, "USD", "10000000"), &open("32.1"))
        .unwrap()
        .quote;
    assert_eq!(q.thb.minor(), 32_100_000_000);
    let q = quote_at(&wants(Side::Sell, "JPY", "10000000"), &open("0.0001"))
        .unwrap()
        .quote;
    assert_eq!(q.foreign.minor_i128(), 10_000_000_000_000);
    assert_eq!(q.thb, money("10000000"));
}

#[test]
fn the_amount_is_above_zero_and_at_most_ten_million() {
    for side in [Side::Buy, Side::Sell] {
        let facts = open("32.1");
        assert!(quote_at(&wants(side, "USD", "10000000"), &facts).is_ok());
        assert!(quote_at(&wants(side, "USD", "0.33"), &facts).is_ok());
        for bad in ["0", "0.00", "10000000.01", "100000000"] {
            assert_eq!(
                quote_at(&wants(side, "USD", bad), &facts),
                Err(E::AmountOrCurrencyInvalid),
                "{side:?} {bad}"
            );
        }
        let unread = QuoteInput {
            amount: None,
            ..wants(side, "USD", "1")
        };
        assert_eq!(quote_at(&unread, &facts), Err(E::AmountOrCurrencyInvalid));
    }
    // the smallest purchase is one hundredth
    assert!(quote_at(&wants(Side::Buy, "USD", "0.01"), &open("32.1")).is_ok());
}

#[test]
fn a_currency_without_a_rate_is_not_quoted() {
    let no_rate = CounterFacts {
        open: true,
        rate: None,
    };
    assert_eq!(
        quote_at(&wants(Side::Buy, "USD", "100"), &no_rate),
        Err(E::AmountOrCurrencyInvalid)
    );
    let unknown = QuoteInput {
        currency: None,
        ..wants(Side::Buy, "USD", "100")
    };
    assert_eq!(
        quote_at(&unknown, &open("32.1")),
        Err(E::AmountOrCurrencyInvalid),
        "a rate without a currency to go with it"
    );
}

#[test]
fn baht_that_buy_less_than_one_hundredth_are_too_small() {
    assert_eq!(
        quote_at(&wants(Side::Sell, "USD", "0.32"), &open("32.6")),
        Err(E::AmountTooSmall)
    );
    let smallest = quote_at(&wants(Side::Sell, "USD", "0.33"), &open("32.6")).unwrap();
    assert_eq!(
        (smallest.quote.foreign, smallest.quote.thb),
        (money("0.01"), money("0.33"))
    );
    // one hundredth of a baht does buy yen
    let yen = quote_at(&wants(Side::Sell, "JPY", "0.01"), &open("0.2195")).unwrap();
    assert_eq!(
        (yen.quote.foreign, yen.quote.thb),
        (money("0.04"), money("0.01"))
    );
}

#[test]
fn a_payment_method_is_one_of_five_and_cash_when_none_is_named() {
    let sell = wants(Side::Sell, "USD", "1000");
    let facts = open("32.6");
    for (sent, held) in [
        (None, PaymentMethod::Cash),
        (Some(""), PaymentMethod::Cash),
        (Some("Cash"), PaymentMethod::Cash),
        (Some("Credit Card"), PaymentMethod::CreditCard),
        (Some("Alipay"), PaymentMethod::Alipay),
        (Some("PromptPay"), PaymentMethod::PromptPay),
        (Some("Bank Transfer"), PaymentMethod::BankTransfer),
    ] {
        let input = QuoteInput {
            payment_method: sent.map(String::from),
            ..sell.clone()
        };
        assert_eq!(
            quote_at(&input, &facts).unwrap().quote.payment_method,
            held,
            "{sent:?}"
        );
    }
    for unknown in [
        "cash",
        "CASH",
        " Cash",
        "SCB Transfer",
        "Bitcoin",
        "Credit card",
    ] {
        assert_eq!(
            quote_at(&paying(sell.clone(), unknown), &facts),
            Err(E::PaymentMethodInvalid),
            "{unknown:?}"
        );
    }
}

#[test]
fn a_customer_who_sells_currency_is_paid_in_cash_or_by_bank_transfer() {
    let buy = wants(Side::Buy, "USD", "100");
    let facts = open("32.1");
    for ok in ["Cash", "Bank Transfer"] {
        assert!(quote_at(&paying(buy.clone(), ok), &facts).is_ok(), "{ok}");
    }
    // a method that exists and one that does not get the same sentence
    for no in ["Credit Card", "Alipay", "PromptPay", "Bitcoin", "cash"] {
        assert_eq!(
            quote_at(&paying(buy.clone(), no), &facts),
            Err(E::BuyPaidInCashOrTransfer),
            "{no}"
        );
    }
}

#[test]
fn a_quote_names_the_first_thing_that_is_wrong() {
    let closed_no_rate = CounterFacts {
        open: false,
        rate: None,
    };
    let no_rate = CounterFacts {
        open: true,
        rate: None,
    };
    let all_wrong = QuoteInput {
        side: None,
        currency: None,
        amount: None,
        payment_method: Some("Bitcoin".into()),
    };
    assert_eq!(quote_at(&all_wrong, &closed_no_rate), Err(E::CounterClosed));
    assert_eq!(quote_at(&all_wrong, &no_rate), Err(E::DirectionRequired));
    let with_side = QuoteInput {
        side: Some(Side::Sell),
        ..all_wrong.clone()
    };
    assert_eq!(
        quote_at(&with_side, &no_rate),
        Err(E::AmountOrCurrencyInvalid)
    );
    assert_eq!(
        quote_at(
            &paying(wants(Side::Sell, "USD", "0.32"), "Bitcoin"),
            &open("32.6")
        ),
        Err(E::AmountTooSmall)
    );
    assert_eq!(
        quote_at(
            &paying(wants(Side::Buy, "USD", "100"), "Bitcoin"),
            &open("32.1")
        ),
        Err(E::BuyPaidInCashOrTransfer)
    );
    // a closed counter is said even to a perfect request
    assert_eq!(
        quote_at(
            &wants(Side::Buy, "USD", "100"),
            &CounterFacts {
                open: false,
                rate: Some(rate("32.1"))
            }
        ),
        Err(E::CounterClosed)
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Time
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn a_quote_is_looked_at_for_three_seconds_before_it_can_be_confirmed() {
    let q = quoted();
    for early in [0, 1, 2_999] {
        assert_eq!(
            refusal(confirm(&q, T0 + early)),
            E::ReviewThreeSeconds,
            "{early} ms"
        );
    }
    let t = confirm(&q, T0 + 3_000).unwrap();
    assert_eq!(t.request.status, S::Waiting);
    assert_eq!(t.event, Some(KioskEventKind::Request));
    assert_eq!(
        t.request,
        KioskRequest {
            status: S::Waiting,
            ..q.clone()
        },
        "nothing but the status changes"
    );
}

#[test]
fn a_clock_that_went_backwards_reads_as_too_early() {
    assert_eq!(
        refusal(confirm(&quoted(), T0 - 5 * MIN)),
        E::ReviewThreeSeconds
    );
    assert_eq!(
        KioskMachine::identity(Some(&quoted()), at(T0 - 5 * MIN))
            .unwrap()
            .status,
        S::Quoted
    );
}

#[test]
fn a_quote_can_be_confirmed_for_five_minutes_and_then_reads_expired() {
    let q = quoted();
    let shown = |now: i64| KioskMachine::identity(Some(&q), at(now)).unwrap().status;

    assert_eq!(shown(T0 + 5 * MIN - 1), S::Quoted);
    assert_eq!(shown(T0 + 5 * MIN), S::Quoted, "exactly five minutes");
    assert_eq!(shown(T0 + 5 * MIN + 1), S::Expired);
    assert_eq!(shown(T0 + 10 * MIN), S::Expired);
    assert_eq!(shown(T0 + 24 * 60 * MIN), S::Expired);

    assert_eq!(
        confirm(&q, T0 + 5 * MIN).unwrap().request.status,
        S::Waiting
    );
    for late in [5 * MIN + 1, 10 * MIN - 1, 10 * MIN, 10 * MIN + 1, 60 * MIN] {
        assert_eq!(
            refusal(confirm(&q, T0 + late)),
            E::QuoteExpired,
            "{late} ms"
        );
    }
    // expiry is read off the clock; the stored status stays what it was
    assert_eq!(q.status, S::Quoted);
}

#[test]
fn only_a_quote_expires() {
    let late = T0 + 24 * 60 * MIN;
    for (held, status) in [
        (waiting(), S::Waiting),
        (accepted(), S::Accepted),
        (id_checked(), S::IdChecked),
        (completed(), S::Completed),
    ] {
        assert_eq!(
            KioskMachine::identity(Some(&held), at(late))
                .unwrap()
                .status,
            status
        );
        assert!(!KioskMachine::is_purgeable(&held, at(late)), "{status:?}");
    }
    // a request confirmed in time is served however long the desk takes
    let t = step(&waiting(), Cmd::Accept, &front1(), &no_facts(), late).unwrap();
    assert_eq!(t.request.status, S::Accepted);
}

#[test]
fn an_unconfirmed_quote_is_swept_away_once_it_is_older_than_ten_minutes() {
    let q = quoted();
    assert!(!KioskMachine::is_purgeable(&q, at(T0)));
    assert!(!KioskMachine::is_purgeable(&q, at(T0 + 5 * MIN + 1)));
    assert!(!KioskMachine::is_purgeable(&q, at(T0 + 10 * MIN - 1)));
    assert!(
        !KioskMachine::is_purgeable(&q, at(T0 + 10 * MIN)),
        "exactly ten minutes"
    );
    assert!(KioskMachine::is_purgeable(&q, at(T0 + 10 * MIN + 1)));
    assert!(!KioskMachine::is_purgeable(&q, at(T0 - MIN)));
}

#[test]
fn a_published_receipt_can_be_read_for_just_under_ten_minutes() {
    let published = at(T0);
    assert!(KioskMachine::receipt_readable(published, at(T0)));
    assert!(KioskMachine::receipt_readable(
        published,
        at(T0 + 10 * MIN - 1)
    ));
    assert!(
        !KioskMachine::receipt_readable(published, at(T0 + 10 * MIN)),
        "exactly ten minutes"
    );
    assert!(!KioskMachine::receipt_readable(
        published,
        at(T0 + 10 * MIN + 1)
    ));
    // published "later than now": the clock moved; it is readable
    assert!(KioskMachine::receipt_readable(published, at(T0 - MIN)));
}

#[test]
fn time_is_milliseconds_and_orders_as_such() {
    assert_eq!(at(T0).epoch_millis(), T0);
    assert_eq!(at(-1).epoch_millis(), -1);
    assert!(at(T0) < at(T0 + 1));
    assert_eq!(at(7), KioskTime::from_epoch_millis(7));
}

// ═════════════════════════════════════════════════════════════════════════════
// Confirm
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn the_counter_and_the_rate_are_asked_again_at_confirm() {
    let q = quoted();
    let now = T0 + 10 * SEC;
    let with = |facts: CounterFacts| step(&q, Cmd::Confirm, &customer(), &facts, now);

    assert!(with(open("32.1")).is_ok());
    assert!(
        with(open("32.1000")).is_ok(),
        "the same rate written longer"
    );
    for moved in ["32.1001", "32.0999", "32.6", "1"] {
        assert_eq!(
            refusal(with(open(moved))),
            E::RateChangedOrClosed,
            "{moved}"
        );
    }
    assert_eq!(
        refusal(with(CounterFacts {
            open: true,
            rate: None
        })),
        E::RateChangedOrClosed,
        "the currency is no longer offered"
    );
    assert_eq!(
        refusal(with(CounterFacts {
            open: false,
            rate: Some(rate("32.1"))
        })),
        E::RateChangedOrClosed,
        "the counter closed"
    );
    // a refused confirm cancels nothing: the same quote is confirmed once the rate is back
    assert!(step(&q, Cmd::Confirm, &customer(), &open("32.1"), now + SEC).is_ok());
}

#[test]
fn too_early_and_too_late_are_said_before_the_counter_is_asked() {
    let q = quoted();
    let closed = no_facts();
    assert_eq!(
        refusal(step(&q, Cmd::Confirm, &customer(), &closed, T0 + SEC)),
        E::ReviewThreeSeconds
    );
    assert_eq!(
        refusal(step(
            &q,
            Cmd::Confirm,
            &customer(),
            &closed,
            T0 + 5 * MIN + 1
        )),
        E::QuoteExpired
    );
    assert_eq!(
        refusal(step(&q, Cmd::Confirm, &customer(), &closed, T0 + 10 * SEC)),
        E::RateChangedOrClosed
    );
}

#[test]
fn confirming_again_answers_the_same_and_writes_nothing() {
    for held in [waiting(), accepted(), id_checked(), completed()] {
        // long after the quote would have run out, with the counter closed and no rate
        let t = step(
            &held,
            Cmd::Confirm,
            &customer(),
            &no_facts(),
            T0 + 3 * 60 * MIN,
        )
        .unwrap();
        assert_eq!(t.request, held);
        assert_eq!(t.event, None);
        // and one millisecond after the quote, where a first confirm would be too early
        let t = step(&held, Cmd::Confirm, &customer(), &no_facts(), T0 + 1).unwrap();
        assert_eq!((t.request, t.event), (held, None));
    }
}

#[test]
fn anyone_may_confirm_whether_signed_in_or_not() {
    for role in ALL_ROLES {
        let t = step(
            &quoted(),
            Cmd::Confirm,
            &staff("someone", role),
            &open("32.1"),
            T0 + 10 * SEC,
        )
        .unwrap();
        assert_eq!(
            (t.request.status, t.request.staff, t.event),
            (S::Waiting, None, Some(KioskEventKind::Request)),
            "{role:?}"
        );
    }
}

#[test]
fn a_request_that_does_not_exist_is_answered_per_action() {
    let now = at(T0);
    let next = |cmd: Cmd, who: &KioskActor| KioskMachine::next(None, &cmd, who, &open("32.1"), now);
    assert_eq!(
        refusal(next(Cmd::Confirm, &customer())),
        E::RequestNotFoundStartAgain
    );
    assert_eq!(
        refusal(next(Cmd::Confirm, &front1())),
        E::RequestNotFoundStartAgain
    );
    assert_eq!(
        refusal(next(Cmd::Accept, &front1())),
        E::AlreadyAssignedOrUnavailable
    );
    assert_eq!(
        refusal(next(checked(true), &front1())),
        E::PassportCheckUnconfirmed
    );
    assert_eq!(KioskMachine::identity(None, now), Err(E::RequestNotFound));
    assert_eq!(
        KioskMachine::check_get(None, &front1()),
        Err(E::AcceptFirst)
    );
    assert_eq!(
        KioskMachine::check_publish(None, &front1()),
        Err(E::AssignedStaffRequired)
    );
    assert_eq!(
        KioskMachine::check_save(None, &matching_save(), &front1()),
        Err(E::NotReadyToSave)
    );
    // who is asking is settled before whether the request exists
    assert_eq!(refusal(next(Cmd::Accept, &customer())), E::StaffRequired);
    assert_eq!(
        KioskMachine::check_get(None, &customer()),
        Err(E::StaffRequired)
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// The desk
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn who_may_call_which_action() {
    use KioskAction as A;
    let open_to_all = [
        A::Rates,
        A::Quote,
        A::Confirm,
        A::Status,
        A::CustomerReceipt,
    ];
    let desk = [
        A::List,
        A::Accept,
        A::Get,
        A::IdentityChecked,
        A::PublishReceipt,
    ];
    let everyone: Vec<KioskActor> = std::iter::once(customer())
        .chain(ALL_ROLES.into_iter().map(|r| staff("someone", r)))
        .collect();

    for who in &everyone {
        for a in open_to_all {
            assert_eq!(KioskMachine::authorize(a, who), Ok(()), "{a:?} {who:?}");
        }
        let role = match who {
            KioskActor::Customer => None,
            KioskActor::Staff { role, .. } => Some(*role),
        };
        // any signed-in person is told how many customers wait
        assert_eq!(
            KioskMachine::authorize(A::Notifications, who),
            if role.is_some() {
                Ok(())
            } else {
                Err(E::StaffRequired)
            },
            "{who:?}"
        );
        let at_the_desk = matches!(
            role,
            Some(KioskRole::Owner | KioskRole::Front | KioskRole::Backend | KioskRole::Manager)
        );
        for a in desk {
            assert_eq!(
                KioskMachine::authorize(a, who),
                if at_the_desk {
                    Ok(())
                } else {
                    Err(E::StaffRequired)
                },
                "{a:?} {who:?}"
            );
        }
        let sets_rates = matches!(role, Some(KioskRole::Owner | KioskRole::Rates));
        for a in [A::Manage, A::Configure] {
            assert_eq!(
                KioskMachine::authorize(a, who),
                if sets_rates {
                    Ok(())
                } else {
                    Err(E::OwnerOrRatesRequired)
                },
                "{a:?} {who:?}"
            );
        }
    }
}

#[test]
fn the_first_of_the_desk_staff_to_accept_holds_the_request() {
    let w = waiting();
    for role in [
        KioskRole::Owner,
        KioskRole::Front,
        KioskRole::Backend,
        KioskRole::Manager,
    ] {
        let t = step(
            &w,
            Cmd::Accept,
            &staff("somebody", role),
            &no_facts(),
            T0 + MIN,
        )
        .unwrap();
        assert_eq!(
            t.request,
            KioskRequest {
                status: S::Accepted,
                staff: Some("somebody".into()),
                ..w.clone()
            },
            "{role:?}"
        );
        assert_eq!(t.event, Some(KioskEventKind::Accepted));
    }
    for who in [
        customer(),
        staff("rates1", KioskRole::Rates),
        staff("finance1", KioskRole::Other),
    ] {
        assert_eq!(
            refusal(step(&w, Cmd::Accept, &who, &no_facts(), T0 + MIN)),
            E::StaffRequired,
            "{who:?}"
        );
    }
    // only a waiting request can be accepted — not a quote, and not one already held, even by its holder
    for held in [quoted(), accepted(), id_checked(), completed()] {
        for who in [front1(), front2()] {
            assert_eq!(
                refusal(step(&held, Cmd::Accept, &who, &no_facts(), T0 + MIN)),
                E::AlreadyAssignedOrUnavailable,
                "{:?} {who:?}",
                held.status
            );
        }
    }
}

#[test]
fn only_the_one_who_accepted_confirms_the_passport_check_and_must_say_so() {
    let a = accepted();
    let now = T0 + MIN;
    let t = step(&a, checked(true), &front1(), &no_facts(), now).unwrap();
    assert_eq!(
        t.request,
        KioskRequest {
            status: S::IdChecked,
            ..a.clone()
        }
    );
    assert_eq!(t.event, Some(KioskEventKind::IdChecked));

    // sent again: the same answer, nothing written
    let again = step(&t.request, checked(true), &front1(), &no_facts(), now).unwrap();
    assert_eq!((again.request, again.event), (t.request.clone(), None));

    assert_eq!(
        refusal(step(&a, checked(false), &front1(), &no_facts(), now)),
        E::PassportCheckUnconfirmed
    );
    for other in [
        front2(),
        staff("owner", KioskRole::Owner),
        staff("manager1", KioskRole::Manager),
    ] {
        assert_eq!(
            refusal(step(&a, checked(true), &other, &no_facts(), now)),
            E::PassportCheckUnconfirmed,
            "{other:?}"
        );
        assert_eq!(
            refusal(step(&t.request, checked(true), &other, &no_facts(), now)),
            E::PassportCheckUnconfirmed,
            "{other:?} on a checked request"
        );
    }
    // the role is asked first, then the tick, then the request
    assert_eq!(
        refusal(step(&a, checked(false), &customer(), &no_facts(), now)),
        E::StaffRequired
    );
    assert_eq!(
        refusal(step(
            &a,
            checked(true),
            &staff("front1", KioskRole::Other),
            &no_facts(),
            now
        )),
        E::StaffRequired,
        "the holder's name under a role the desk does not have"
    );
    for held in [quoted(), waiting(), completed()] {
        assert_eq!(
            refusal(step(&held, checked(true), &front1(), &no_facts(), now)),
            E::PassportCheckUnconfirmed,
            "{:?}",
            held.status
        );
    }
}

#[test]
fn the_desk_opens_a_request_only_for_the_one_who_holds_it() {
    // opened at the desk while it is being served; a receipt is published once the passport is checked
    for (held, get, publish) in [
        (quoted(), false, false),
        (waiting(), false, false),
        (accepted(), true, false),
        (id_checked(), true, true),
        (completed(), false, true),
    ] {
        assert_eq!(
            KioskMachine::check_get(Some(&held), &front1()),
            if get { Ok(()) } else { Err(E::AcceptFirst) },
            "get {:?}",
            held.status
        );
        assert_eq!(
            KioskMachine::check_publish(Some(&held), &front1()),
            if publish {
                Ok(())
            } else {
                Err(E::AssignedStaffRequired)
            },
            "publish {:?}",
            held.status
        );
        for other in [front2(), staff("owner", KioskRole::Owner)] {
            assert_eq!(
                KioskMachine::check_get(Some(&held), &other),
                Err(E::AcceptFirst)
            );
            assert_eq!(
                KioskMachine::check_publish(Some(&held), &other),
                Err(E::AssignedStaffRequired)
            );
        }
        for outsider in [customer(), staff("front1", KioskRole::Rates)] {
            assert_eq!(
                KioskMachine::check_get(Some(&held), &outsider),
                Err(E::StaffRequired)
            );
            assert_eq!(
                KioskMachine::check_publish(Some(&held), &outsider),
                Err(E::StaffRequired)
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// The save
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn a_save_must_be_the_holders_real_and_what_the_customer_was_quoted() {
    let c = id_checked();
    let judge = |change: &dyn Fn(&mut SaveAttempt)| {
        let mut s = matching_save();
        change(&mut s);
        KioskMachine::check_save(Some(&c), &s, &front1())
    };
    assert_eq!(judge(&|_| {}), Ok(()));
    assert_eq!(judge(&|s| s.live = false), Err(E::SaveMustBeLive));
    assert_eq!(
        judge(&|s| s.currency = code("EUR")),
        Err(E::SaveDiffersFromQueue)
    );
    assert_eq!(
        judge(&|s| s.side = Side::Sell),
        Err(E::SaveDiffersFromQueue)
    );
    assert_eq!(
        judge(&|s| s.amount = money("100.01")),
        Err(E::SaveDiffersFromQueue)
    );
    assert_eq!(
        judge(&|s| s.amount = money("3210")),
        Err(E::SaveDiffersFromQueue),
        "the baht figure is not the amount"
    );
    for other in ["Bank Transfer", "cash", "SCB Transfer", ""] {
        assert_eq!(
            judge(&|s| s.method = other.into()),
            Err(E::SaveMethodDiffers),
            "{other:?}"
        );
    }
    // in this order: practice, the figures, the method
    assert_eq!(
        judge(&|s| {
            s.live = false;
            s.currency = code("EUR");
            s.method = "Alipay".into();
        }),
        Err(E::SaveMustBeLive)
    );
    assert_eq!(
        judge(&|s| {
            s.currency = code("EUR");
            s.method = "Alipay".into();
        }),
        Err(E::SaveDiffersFromQueue)
    );
    // any submission id may save a request that is checked and not yet completed
    assert_eq!(judge(&|s| s.submission_id = "SUB-OTHER".into()), Ok(()));
}

#[test]
fn a_save_is_refused_until_the_passport_is_checked_and_to_everyone_but_the_holder() {
    for early in [quoted(), waiting(), accepted()] {
        assert_eq!(
            KioskMachine::check_save(Some(&early), &matching_save(), &front1()),
            Err(E::NotReadyToSave),
            "{:?}",
            early.status
        );
    }
    let c = id_checked();
    for other in [front2(), staff("owner", KioskRole::Owner), customer()] {
        assert_eq!(
            KioskMachine::check_save(Some(&c), &matching_save(), &other),
            Err(E::NotReadyToSave),
            "{other:?}"
        );
    }
    // the save has its own gates for roles; here only the name counts
    assert_eq!(
        KioskMachine::check_save(
            Some(&c),
            &matching_save(),
            &staff("front1", KioskRole::Other)
        ),
        Ok(())
    );
    // being someone else is said before anything about the save itself
    let practice = SaveAttempt {
        live: false,
        ..matching_save()
    };
    assert_eq!(
        KioskMachine::check_save(Some(&c), &practice, &front2()),
        Err(E::NotReadyToSave)
    );
}

#[test]
fn saving_completes_a_checked_request_and_gives_it_its_document_number() {
    let c = id_checked();
    let t = KioskMachine::complete(&c, "SUB-1", "XC-20261006-000042").unwrap();
    assert_eq!(
        t.request,
        KioskRequest {
            status: S::Completed,
            transaction: Some(TransactionLink {
                submission_id: "SUB-1".into(),
                document_ref: "XC-20261006-000042".into(),
            }),
            ..c.clone()
        }
    );
    assert_eq!(t.event, Some(KioskEventKind::Completed));
    assert_eq!(
        KioskMachine::identity(Some(&t.request), at(T0 + MIN))
            .unwrap()
            .transaction_no
            .as_deref(),
        Some("XC-20261006-000042")
    );
}

#[test]
fn a_completed_request_is_saved_again_only_by_the_save_that_completed_it() {
    let done = completed();
    assert_eq!(
        KioskMachine::check_save(Some(&done), &matching_save(), &front1()),
        Ok(())
    );
    let another = SaveAttempt {
        submission_id: "SUB-2".into(),
        ..matching_save()
    };
    assert_eq!(
        KioskMachine::check_save(Some(&done), &another, &front1()),
        Err(E::NotReadyToSave)
    );
    assert_eq!(
        KioskMachine::check_save(Some(&done), &matching_save(), &front2()),
        Err(E::NotReadyToSave)
    );
    // the retry is still held to the other rules
    let practice = SaveAttempt {
        live: false,
        ..matching_save()
    };
    assert_eq!(
        KioskMachine::check_save(Some(&done), &practice, &front1()),
        Err(E::SaveMustBeLive)
    );

    // completing again changes nothing, whichever save says so
    for (submission, document) in [
        ("SUB-1", "XC-20261006-000001"),
        ("SUB-2", "XC-20261006-000099"),
    ] {
        let t = KioskMachine::complete(&done, submission, document).unwrap();
        assert_eq!((t.request, t.event), (done.clone(), None), "{submission}");
    }
}

#[test]
fn a_completed_request_without_a_link_admits_no_save() {
    // not reachable through the machine; a row could be stored like this
    let odd = KioskRequest {
        transaction: None,
        ..completed()
    };
    assert_eq!(
        KioskMachine::check_save(Some(&odd), &matching_save(), &front1()),
        Err(E::NotReadyToSave)
    );
}

#[test]
fn the_first_link_to_a_saved_document_is_kept() {
    // not reachable through the machine: a checked request that already carries a link
    let first = TransactionLink {
        submission_id: "SUB-OLD".into(),
        document_ref: "XC-20261006-000001".into(),
    };
    let odd = KioskRequest {
        transaction: Some(first.clone()),
        ..id_checked()
    };
    let t = KioskMachine::complete(&odd, "SUB-NEW", "XC-20261006-000002").unwrap();
    assert_eq!(t.request.status, S::Completed);
    assert_eq!(t.request.transaction, Some(first));
    assert_eq!(t.event, Some(KioskEventKind::Completed));
}

#[test]
fn a_request_that_was_never_checked_is_not_completed() {
    // v2 wrote the link and left the status; here it is refused and nothing is written
    for early in [quoted(), waiting(), accepted()] {
        assert_eq!(
            refusal(KioskMachine::complete(
                &early,
                "SUB-1",
                "XC-20261006-000001"
            )),
            E::NotReadyToSave,
            "{:?}",
            early.status
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// What is shown and said
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn the_customer_is_shown_a_queue_number_an_order_number_and_later_the_document_number() {
    let q = quoted();
    assert_eq!(
        KioskMachine::identity(Some(&q), at(T0 + SEC)),
        Ok(RequestIdentity {
            id: ID.into(),
            queue: "K-3FA9C1D2".into(),
            order_no: "ORD-3FA9C1D2-7E57-4000-8000-00000000BEEF".into(),
            transaction_no: None,
            status: S::Quoted,
        })
    );
    let short = KioskRequest {
        id: "ab1".into(),
        ..q.clone()
    };
    let shown = KioskMachine::identity(Some(&short), at(T0)).unwrap();
    assert_eq!(
        (shown.queue.as_str(), shown.order_no.as_str()),
        ("K-AB1", "ORD-AB1")
    );
    let done = KioskMachine::identity(Some(&completed()), at(T0 + MIN)).unwrap();
    assert_eq!(
        (done.status, done.transaction_no.as_deref()),
        (S::Completed, Some("XC-20261006-000001"))
    );
}

#[test]
fn a_stored_status_of_expired_is_treated_as_a_quote_that_ran_out() {
    // the machine never stores it; a row could
    let stored = KioskRequest {
        status: S::Expired,
        ..quoted()
    };
    assert_eq!(
        refusal(confirm(&stored, T0 + 10 * SEC)),
        E::QuoteExpired,
        "even within the five minutes"
    );
    assert_eq!(
        KioskMachine::identity(Some(&stored), at(T0 + 10 * SEC))
            .unwrap()
            .status,
        S::Expired
    );
    assert_eq!(
        refusal(step(
            &stored,
            Cmd::Accept,
            &front1(),
            &no_facts(),
            T0 + 10 * SEC
        )),
        E::AlreadyAssignedOrUnavailable
    );
    assert_eq!(
        KioskMachine::check_save(Some(&stored), &matching_save(), &front1()),
        Err(E::NotReadyToSave)
    );
}

#[test]
fn names_are_the_ones_the_screens_and_the_audit_log_use() {
    assert_eq!(
        ALL_STATUSES.map(S::as_str),
        [
            "QUOTED",
            "WAITING",
            "ACCEPTED",
            "ID_CHECKED",
            "COMPLETED",
            "EXPIRED"
        ]
    );
    assert_eq!(
        ALL_METHODS.map(PaymentMethod::as_str),
        [
            "Cash",
            "Credit Card",
            "Alipay",
            "PromptPay",
            "Bank Transfer"
        ]
    );
    for m in ALL_METHODS {
        assert_eq!(PaymentMethod::parse(m.as_str()), Some(m));
    }
    for unknown in ["", "cash", "Cash ", "SCB Transfer", "PROMPTPAY"] {
        assert_eq!(PaymentMethod::parse(unknown), None, "{unknown:?}");
    }
    assert_eq!(
        [
            KioskEventKind::Request,
            KioskEventKind::Accepted,
            KioskEventKind::IdChecked,
            KioskEventKind::Completed
        ]
        .map(KioskEventKind::as_str),
        [
            "KIOSK_REQUEST",
            "KIOSK_ACCEPTED",
            "KIOSK_ID_CHECKED",
            "KIOSK_COMPLETED"
        ]
    );
    assert_eq!(
        ALL_ACTIONS.map(KioskAction::as_str),
        [
            "rates",
            "quote",
            "confirm",
            "status",
            "customerReceipt",
            "notifications",
            "list",
            "accept",
            "get",
            "identityChecked",
            "publishReceipt",
            "manage",
            "configure"
        ]
    );
}

#[test]
fn every_refusal_says_v2s_sentence() {
    let sentences = [
        (
            E::CounterClosed,
            "Counter is closed. Please contact our staff.",
        ),
        (E::DirectionRequired, "Select an exchange direction."),
        (
            E::AmountOrCurrencyInvalid,
            "Please enter a valid amount and currency.",
        ),
        (E::AmountTooSmall, "Amount is too small."),
        (
            E::BuyPaidInCashOrTransfer,
            "Select cash or bank transfer to receive THB.",
        ),
        (E::PaymentMethodInvalid, "Invalid payment method."),
        (
            E::RequestNotFoundStartAgain,
            "Request not found. Please start again.",
        ),
        (
            E::QuoteExpired,
            "Quote expired. Please check the rate again.",
        ),
        (
            E::ReviewThreeSeconds,
            "Please review your quote for 3 seconds.",
        ),
        (
            E::RateChangedOrClosed,
            "Rate changed or counter closed. Please start again.",
        ),
        (E::RequestNotFound, "Request not found."),
        (E::StaffRequired, "Staff access required."),
        (E::OwnerOrRatesRequired, "Owner or Rates access required."),
        (
            E::PassportCheckUnconfirmed,
            "The assigned employee must confirm the passport check.",
        ),
        (E::AssignedStaffRequired, "Assigned staff required."),
        (
            E::AcceptFirst,
            "Accept this request using your own account first.",
        ),
        (
            E::AlreadyAssignedOrUnavailable,
            "This request is already assigned or unavailable.",
        ),
        (E::NotReadyToSave, "คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว"),
        (E::SaveMustBeLive, "คิวต้องบันทึกเป็นรายการจริง"),
        (E::SaveDiffersFromQueue, "สกุลเงินและจำนวนต้องตรงกับคิว"),
        (E::SaveMethodDiffers, "ช่องทางต้องตรงกับที่ลูกค้าเลือก"),
    ];
    assert_eq!(sentences.len(), 21);
    for (error, sentence) in sentences {
        assert_eq!(error.to_string(), sentence, "{error:?}");
    }
}

#[test]
fn what_the_machine_is_handed_stays_as_it_was() {
    let input = paying(wants(Side::Sell, "EUR", "5000"), "PromptPay");
    let facts = open("38.2");
    let (input_was, facts_was) = (input.clone(), facts);
    let made = KioskMachine::quote(&input, &facts, ID, at(T0)).unwrap();
    assert_eq!((&input, &facts), (&input_was, &facts_was));

    let was = made.clone();
    let t = step(&made, Cmd::Confirm, &customer(), &facts, T0 + 5 * SEC).unwrap();
    assert_eq!(made, was, "confirm returned a new request");
    assert_ne!(t.request, was);

    let w = t.request;
    let w_was = w.clone();
    let a = step(&w, Cmd::Accept, &front1(), &no_facts(), T0 + MIN).unwrap();
    assert_eq!(w, w_was, "accept returned a new request");

    let a_was = a.request.clone();
    let c = step(&a.request, checked(true), &front1(), &no_facts(), T0 + MIN).unwrap();
    assert_eq!(
        a.request, a_was,
        "the passport check returned a new request"
    );

    let c_was = c.request.clone();
    let done = KioskMachine::complete(&c.request, "SUB-1", "XC-20261006-000001").unwrap();
    assert_eq!(c.request, c_was, "completing returned a new request");
    assert_eq!(done.request.status, S::Completed);
}

// ═════════════════════════════════════════════════════════════════════════════
// Every answer v2 gave
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn every_quote_v2_made_is_made_the_same_or_exactly() {
    let fx = fixture();
    for (n, q) in fx.quotes.iter().enumerate() {
        play(
            &fx,
            q,
            None,
            q.now.expect("now"),
            &format!("quote {}", n + 1),
        );
    }
}

#[test]
fn every_action_by_everyone_from_every_state_gets_v2s_answer() {
    let fx = fixture();
    for state in &fx.states {
        let held = state.request.as_ref().map(request);
        for (n, case) in state.cases.iter().enumerate() {
            assert!(
                case.now.is_none() && case.before.is_none(),
                "{}: a matrix case carries its own state",
                state.state
            );
            play(
                &fx,
                case,
                held.as_ref(),
                state.now,
                &format!("state {} ({}) case {}", state.state, state.what, n + 1),
            );
        }
    }
}

#[test]
fn every_story_v2_played_plays_the_same() {
    let fx = fixture();
    for story in &fx.stories {
        // the one request the story is about, as the machine left it
        let mut held: Option<KioskRequest> = None;
        for (n, s) in story.steps.iter().enumerate() {
            let label = format!("story {} step {}", story.name, n + 1);
            let now = s.now.expect("now");
            if s.action == "quote" {
                let made = play(&fx, s, None, now, &label);
                if held.is_none() {
                    held = made;
                }
                continue;
            }
            assert_eq!(
                held,
                s.before.as_ref().map(request),
                "{label}: the request is not where v2's was before this step"
            );
            held = play(&fx, s, held.as_ref(), now, &label);
        }
    }
}

#[test]
fn the_counter_and_the_rate_at_confirm_get_v2s_answer() {
    let fx = fixture();
    for (n, c) in fx.confirms.iter().enumerate() {
        let held = c.before.as_ref().map(request);
        play(
            &fx,
            c,
            held.as_ref(),
            c.now.expect("now"),
            &format!("confirm {}", n + 1),
        );
    }
}

#[test]
fn every_save_v2_judged_is_judged_the_same() {
    let fx = fixture();
    for (n, s) in fx.saves.iter().enumerate() {
        let held = s.before.as_ref().map(request);
        play(
            &fx,
            s,
            held.as_ref(),
            s.now.expect("now"),
            &format!("save {}", n + 1),
        );
    }
}

#[test]
fn who_may_call_which_action_is_as_in_v2() {
    let fx = fixture();
    for a in &fx.access {
        let got = KioskMachine::authorize(action(&a.action), &actor(&fx, &a.actor));
        assert_eq!(
            got.err().map(|e| e.to_string()),
            a.error,
            "{} by {}",
            a.action,
            a.actor
        );
    }
}

#[test]
fn quotes_are_swept_and_receipts_run_out_when_v2s_did() {
    let fx = fixture();
    for s in &fx.sweep {
        assert_eq!(
            KioskMachine::is_purgeable(&request(&s.request), at(s.now)),
            s.swept,
            "{} at age {} ms",
            s.request.status,
            s.now - s.request.created
        );
    }
    for r in &fx.receipt {
        assert_eq!(
            KioskMachine::receipt_readable(at(r.published), at(r.now)),
            r.readable,
            "receipt at age {} ms",
            r.now - r.published
        );
    }
}

#[test]
fn the_fixture_holds_the_cases_it_was_generated_with() {
    let fx = fixture();
    let matrix: usize = fx.states.iter().map(|s| s.cases.len()).sum();
    let steps: usize = fx.stories.iter().map(|s| s.steps.len()).sum();
    assert_eq!(
        (
            fx.people.len(),
            fx.access.len(),
            fx.quotes.len(),
            fx.states.len(),
            matrix,
            fx.confirms.len(),
            fx.saves.len(),
            fx.stories.len(),
            steps,
            fx.sweep.len(),
            fx.receipt.len(),
        ),
        (9, 117, 219, 10, 872, 12, 22, 7, 68, 9, 6)
    );
    assert_eq!(
        fx.quotes.iter().filter(|q| q.error.is_some()).count(),
        59,
        "refused quotes"
    );
    assert_eq!(
        fx.quotes.iter().filter(|q| q.v2_differs_from_exact).count(),
        12,
        "quotes where v2's arithmetic was not exact"
    );
    let on_purpose = fx
        .states
        .iter()
        .flat_map(|s| &s.cases)
        .filter(|c| c.v2_differs_on_purpose)
        .count();
    assert_eq!(on_purpose, 56, "answers that differ from v2 on purpose");
    // every sentence of the machine is one v2 said at least once
    let mut said: Vec<&str> = fx
        .quotes
        .iter()
        .chain(fx.states.iter().flat_map(|s| &s.cases))
        .chain(&fx.confirms)
        .chain(&fx.saves)
        .chain(fx.stories.iter().flat_map(|s| &s.steps))
        .filter_map(|s| s.error.as_deref())
        .chain(fx.access.iter().filter_map(|a| a.error.as_deref()))
        .collect();
    said.sort_unstable();
    said.dedup();
    assert_eq!(said.len(), 21);
}
