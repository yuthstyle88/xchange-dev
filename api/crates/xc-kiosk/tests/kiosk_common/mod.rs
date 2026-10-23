//! Shared by the kiosk tests: the v2 fixture, the conversions it needs, and the one
//! function that plays a recorded step against the machine.
//! Provided with ticket T12a — do not edit. (POL's author removed `Access`: the role table
//! moved to `Policy`, and `v2_kiosk_cases.json` no longer carries `access`.)
//!
//! Edited by the author of ticket #756 (RE-4b, `docs/design/blueprint/rates.md` §4.9 R1, R4): the
//! machine is handed the published line it prices from (`CounterFacts::line`, a `CounterLine`),
//! and a quote keeps that line's id and `unit_size`. The v2 fixture prices every currency per 1
//! with two-place amounts, so `line_at` gives each fixture rate a line with `unit_size` 1 and
//! minor unit 2, and an id that is the same for the same rate (`line_of`): v2's "the rate
//! changed" is RE-4b's "the line changed", with the same answers.
#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;

use serde::Deserialize;
use xc_kernel::money::{CurrencyCode, ForeignAmount, MinorUnit, Money, Rate, Side, UnitSize};

// Every kiosk path the tests name is in this one block.
pub use xc_kiosk::domain::kiosk::commands::kiosk_command::KioskCommand;
pub use xc_kiosk::domain::kiosk::commands::quote_input::QuoteInput;
pub use xc_kiosk::domain::kiosk::entities::kiosk_machine::KioskMachine;
pub use xc_kiosk::domain::kiosk::entities::kiosk_request::KioskRequest;
pub use xc_kiosk::domain::kiosk::value_objects::counter_facts::{
    CounterFacts, CounterLine, LineId,
};
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_action::KioskAction;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_error::KioskError;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_event_kind::KioskEventKind;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_role::KioskRole;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_status::KioskStatus;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_time::KioskTime;
pub use xc_kiosk::domain::kiosk::value_objects::payment_method::PaymentMethod;
pub use xc_kiosk::domain::kiosk::value_objects::request_identity::RequestIdentity;
pub use xc_kiosk::domain::kiosk::value_objects::save_attempt::SaveAttempt;
pub use xc_kiosk::domain::kiosk::value_objects::transaction_link::TransactionLink;
pub use xc_kiosk::domain::kiosk::value_objects::transition::Transition;

// ── the fixture ──────────────────────────────────────────────────────────────

#[derive(Deserialize, Clone)]
pub struct Person {
    pub username: String,
    pub role: String,
}

#[derive(Deserialize, Clone)]
pub struct Facts {
    pub open: bool,
    pub rate: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuoteJson {
    #[serde(rename = "type")]
    pub side: String,
    pub currency: String,
    pub rate: String,
    pub foreign: String,
    pub thb: String,
    pub input_amount: String,
    pub payment_method: String,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LinkJson {
    pub submission_id: String,
    pub document_ref: String,
}

#[derive(Deserialize, Clone)]
pub struct RequestJson {
    pub id: String,
    pub status: String,
    pub created: i64,
    pub staff: Option<String>,
    pub quote: QuoteJson,
    pub transaction: Option<LinkJson>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IdentityJson {
    pub id: String,
    pub queue: String,
    pub order_no: String,
    pub transaction_no: Option<String>,
    pub status: String,
}

/// What the customer typed and chose, as the screen sent it.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InputJson {
    #[serde(rename = "type")]
    pub side: Option<String>,
    pub currency: Option<String>,
    pub amount: Option<String>,
    pub payment_method: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SaveJson {
    pub live: bool,
    #[serde(rename = "type")]
    pub side: String,
    pub currency: String,
    pub amount: String,
    pub method: String,
    pub submission_id: String,
}

/// One thing v2 was asked, and what it answered.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub actor: String,
    pub action: String,
    /// Absent on a matrix case: the state carries the moment.
    pub now: Option<i64>,
    pub note: Option<String>,
    // quote
    pub input: Option<InputJson>,
    pub facts: Option<Facts>,
    pub new_id: Option<String>,
    pub request: Option<RequestJson>,
    #[serde(default)]
    pub v2_differs_from_exact: bool,
    // everything else
    pub before: Option<RequestJson>,
    pub confirmed: Option<bool>,
    pub save: Option<SaveJson>,
    pub submission_id: Option<String>,
    pub document_ref: Option<String>,
    // the answer
    pub error: Option<String>,
    pub after: Option<RequestJson>,
    #[serde(default)]
    pub unchanged: bool,
    pub event: Option<String>,
    pub identity: Option<IdentityJson>,
    #[serde(default)]
    pub v2_differs_on_purpose: bool,
}

#[derive(Deserialize)]
pub struct MatrixState {
    pub state: String,
    pub what: String,
    pub now: i64,
    pub request: Option<RequestJson>,
    pub cases: Vec<Step>,
}

#[derive(Deserialize)]
pub struct Story {
    pub name: String,
    pub what: String,
    pub steps: Vec<Step>,
}

#[derive(Deserialize)]
pub struct Sweep {
    pub request: RequestJson,
    pub now: i64,
    pub swept: bool,
}

#[derive(Deserialize)]
pub struct Receipt {
    pub published: i64,
    pub now: i64,
    pub readable: bool,
}

#[derive(Deserialize)]
pub struct Fixture {
    pub people: BTreeMap<String, Option<Person>>,
    pub quotes: Vec<Step>,
    pub states: Vec<MatrixState>,
    pub confirms: Vec<Step>,
    pub saves: Vec<Step>,
    pub stories: Vec<Story>,
    pub sweep: Vec<Sweep>,
    pub receipt: Vec<Receipt>,
}

pub fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/v2_kiosk_cases.json"))
        .expect("v2_kiosk_cases.json")
}

// ── from fixture text to the machine's types ─────────────────────────────────

/// A figure written as `"100"`, `"100.5"` or `"100.50"`. Not `Money::parse`: a figure the
/// machine computed may pass the cap on a figure a person enters.
pub fn money(text: &str) -> Money {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    let plain = !whole.is_empty()
        && frac.len() <= 2
        && whole
            .bytes()
            .chain(frac.bytes())
            .all(|b| b.is_ascii_digit());
    assert!(plain, "money {text:?}");
    let minor: i64 = format!("{whole}{frac:0<2}")
        .parse()
        .unwrap_or_else(|e| panic!("money {text:?}: {e}"));
    Money::from_minor(minor).unwrap_or_else(|e| panic!("money {text:?}: {e}"))
}

pub fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap_or_else(|e| panic!("rate {text:?}: {e}"))
}

pub fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap_or_else(|_| panic!("currency {text:?}"))
}

pub fn at(millis: i64) -> KioskTime {
    KioskTime::from_epoch_millis(millis)
}

pub fn side(text: &str) -> Side {
    side_as_sent(text).unwrap_or_else(|| panic!("side {text:?}"))
}

/// The direction as v2 reads it: exactly `BUY` or `SELL`, anything else is none.
pub fn side_as_sent(text: &str) -> Option<Side> {
    match text {
        "BUY" => Some(Side::Buy),
        "SELL" => Some(Side::Sell),
        _ => None,
    }
}

pub fn status(text: &str) -> KioskStatus {
    ALL_STATUSES
        .into_iter()
        .find(|s| s.as_str() == text)
        .unwrap_or_else(|| panic!("status {text:?}"))
}

/// Amended by the author of KC-D (claude/studio-claude-10, `kiosk-cancel.md` §3.1): `CANCELLED`
/// is a stored status.
pub const ALL_STATUSES: [KioskStatus; 7] = [
    KioskStatus::Quoted,
    KioskStatus::Waiting,
    KioskStatus::Accepted,
    KioskStatus::IdChecked,
    KioskStatus::Completed,
    KioskStatus::Expired,
    KioskStatus::Cancelled,
];

/// Amended by the author of KC-S #4176 (claude/studio-claude-10, `kiosk-cancel.md` §3.4):
/// `cancel` and `customerCancel`.
pub const ALL_ACTIONS: [KioskAction; 15] = [
    KioskAction::Rates,
    KioskAction::Quote,
    KioskAction::Confirm,
    KioskAction::Status,
    KioskAction::CustomerReceipt,
    KioskAction::Notifications,
    KioskAction::List,
    KioskAction::Accept,
    KioskAction::Get,
    KioskAction::IdentityChecked,
    KioskAction::PublishReceipt,
    KioskAction::Manage,
    KioskAction::Configure,
    KioskAction::Cancel,
    KioskAction::CustomerCancel,
];

pub const ALL_METHODS: [PaymentMethod; 5] = [
    PaymentMethod::Cash,
    PaymentMethod::CreditCard,
    PaymentMethod::Alipay,
    PaymentMethod::PromptPay,
    PaymentMethod::BankTransfer,
];

pub const ALL_ROLES: [KioskRole; 6] = [
    KioskRole::Owner,
    KioskRole::Front,
    KioskRole::Backend,
    KioskRole::Manager,
    KioskRole::Rates,
    KioskRole::Other,
];

pub fn action(text: &str) -> KioskAction {
    ALL_ACTIONS
        .into_iter()
        .find(|a| a.as_str() == text)
        .unwrap_or_else(|| panic!("action {text:?}"))
}

pub fn method(text: &str) -> PaymentMethod {
    PaymentMethod::parse(text).unwrap_or_else(|| panic!("payment method {text:?}"))
}

/// A v2 role as the kiosk knows it: the five it names, and `Other` for the rest.
pub fn role(text: &str) -> KioskRole {
    match text {
        "OWNER" => KioskRole::Owner,
        "FRONT" => KioskRole::Front,
        "BACKEND" => KioskRole::Backend,
        "MANAGER" => KioskRole::Manager,
        "RATES" => KioskRole::Rates,
        _ => KioskRole::Other,
    }
}

pub fn staff(username: &str, role: KioskRole) -> KioskActor {
    KioskActor::Staff {
        username: username.into(),
        role,
    }
}

pub fn actor(fx: &Fixture, name: &str) -> KioskActor {
    match fx
        .people
        .get(name)
        .unwrap_or_else(|| panic!("nobody called {name}"))
    {
        None => KioskActor::Customer,
        Some(p) => staff(&p.username, role(&p.role)),
    }
}

/// The id of the line a fixture rate stands for: the same rate, the same line (#756).
pub fn line_of(text: &str) -> LineId {
    let mut value = rate(text).value();
    value.rescale(8);
    LineId(u128::try_from(value.mantissa()).expect("a rate is positive"))
}

/// A line priced per 1, with two-place amounts, as v2 priced every currency (#756).
pub fn line_at(text: &str) -> CounterLine {
    CounterLine {
        line_id: line_of(text),
        rate: rate(text),
        unit_size: UnitSize::ONE,
        minor_unit: MinorUnit::TWO,
    }
}

pub fn facts(f: &Facts) -> CounterFacts {
    CounterFacts {
        open: f.open,
        line: f.rate.as_deref().map(line_at),
    }
}

pub fn request(r: &RequestJson) -> KioskRequest {
    KioskRequest {
        id: r.id.clone(),
        status: status(&r.status),
        created: at(r.created),
        staff: r.staff.clone(),
        quote: KioskQuote {
            side: side(&r.quote.side),
            currency: code(&r.quote.currency),
            rate: rate(&r.quote.rate),
            unit_size: UnitSize::ONE,
            line_id: Some(line_of(&r.quote.rate)),
            foreign: money(&r.quote.foreign),
            thb: money(&r.quote.thb),
            input_amount: money(&r.quote.input_amount),
            payment_method: method(&r.quote.payment_method),
        },
        transaction: r.transaction.as_ref().map(|t| TransactionLink {
            submission_id: t.submission_id.clone(),
            document_ref: t.document_ref.clone(),
        }),
    }
}

pub fn identity(i: &IdentityJson) -> RequestIdentity {
    RequestIdentity {
        id: i.id.clone(),
        queue: i.queue.clone(),
        order_no: i.order_no.clone(),
        transaction_no: i.transaction_no.clone(),
        status: status(&i.status),
    }
}

/// The input as the caller of the machine hands it over: what cannot be read is `None`.
pub fn quote_input(i: &InputJson) -> QuoteInput {
    QuoteInput {
        side: i.side.as_deref().and_then(side_as_sent),
        currency: i
            .currency
            .as_deref()
            .and_then(|c| CurrencyCode::parse(c).ok()),
        amount: i.amount.as_deref().and_then(|a| Money::parse(a).ok()),
        payment_method: i.payment_method.clone(),
    }
}

pub fn save(s: &SaveJson) -> SaveAttempt {
    let m = money(&s.amount);
    SaveAttempt {
        live: s.live,
        side: side(&s.side),
        currency: code(&s.currency),
        amount: ForeignAmount::from_decimal(rust_decimal::Decimal::from_i128_with_scale(
            m.minor_i128(),
            2,
        )),
        method: s.method.clone(),
        submission_id: s.submission_id.clone(),
    }
}

/// Facts for the commands that must not read them: a closed counter with no rate.
pub fn no_facts() -> CounterFacts {
    CounterFacts {
        open: false,
        line: None,
    }
}

// ── playing a recorded step ──────────────────────────────────────────────────

fn told<T: std::fmt::Debug>(got: Result<T, KioskError>, step: &Step, label: &str) -> Option<T> {
    match (got, &step.error) {
        (Ok(v), None) => Some(v),
        (Err(e), Some(sentence)) => {
            assert_eq!(&e.to_string(), sentence, "{label}: another refusal");
            None
        }
        (Ok(v), Some(sentence)) => panic!("{label}: v2 refused with {sentence:?}, got {v:?}"),
        (Err(e), None) => panic!("{label}: v2 accepted, got {e:?} ({e})"),
    }
}

/// Plays one recorded step and holds the machine to v2's answer.
///
/// `before` is the request the step is about (`None`: no request has that id) and `now`
/// the moment it arrives. Returns the request as it is afterwards.
pub fn play(
    fx: &Fixture,
    step: &Step,
    before: Option<&KioskRequest>,
    now: i64,
    label: &str,
) -> Option<KioskRequest> {
    let label = format!(
        "{label} · {} {}{} at {now}",
        step.actor,
        step.action,
        step.note
            .as_deref()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default()
    );
    let label = label.as_str();
    let who = actor(fx, &step.actor);
    let untouched = before.cloned();
    let moment = at(now);

    let after = match step.action.as_str() {
        "quote" => {
            let input = quote_input(step.input.as_ref().expect("input"));
            let input_was = input.clone();
            let f = facts(step.facts.as_ref().expect("facts"));
            let new_id = step.new_id.as_deref().expect("newId");
            let made = told(KioskMachine::quote(&input, &f, new_id, moment), step, label);
            assert_eq!(input, input_was, "{label}: the input was changed");
            if let Some(made) = &made {
                let expected = request(step.request.as_ref().expect("request"));
                assert_eq!(made, &expected, "{label}: another quote");
                assert_eq!(
                    KioskMachine::identity(Some(made), moment),
                    Ok(identity(step.identity.as_ref().expect("identity"))),
                    "{label}: another identity"
                );
            }
            made
        }
        "confirm" | "accept" | "identityChecked" => {
            let command = match step.action.as_str() {
                "confirm" => KioskCommand::Confirm,
                "accept" => KioskCommand::Accept,
                _ => KioskCommand::IdentityChecked {
                    confirmed: step.confirmed.unwrap_or(false),
                },
            };
            // only a confirm reads the counter; the others are handed a closed one
            let f = match (&step.facts, step.action.as_str()) {
                (Some(f), "confirm") => facts(f),
                (None, "confirm") => panic!("{label}: a confirm without facts"),
                _ => no_facts(),
            };
            let got = told(
                KioskMachine::next(before, &command, &who, &f, moment),
                step,
                label,
            );
            match got {
                None => untouched.clone(),
                Some(t) => {
                    let expected = expected_after(step, before, label);
                    assert_eq!(
                        Some(&t.request),
                        expected.as_ref(),
                        "{label}: another request"
                    );
                    assert_eq!(
                        t.event.map(KioskEventKind::as_str),
                        step.event.as_deref(),
                        "{label}: another audit event"
                    );
                    if let Some(i) = &step.identity {
                        assert_eq!(
                            KioskMachine::identity(Some(&t.request), moment),
                            Ok(identity(i)),
                            "{label}: another identity"
                        );
                    }
                    Some(t.request)
                }
            }
        }
        "status" => {
            let got = told(KioskMachine::identity(before, moment), step, label);
            if let Some(got) = got {
                assert_eq!(
                    got,
                    identity(step.identity.as_ref().expect("identity")),
                    "{label}: another identity"
                );
            }
            untouched.clone()
        }
        "get" => {
            told(KioskMachine::check_get(before, &who), step, label);
            untouched.clone()
        }
        "publishReceipt" => {
            told(KioskMachine::check_publish(before, &who), step, label);
            untouched.clone()
        }
        "checkSave" => {
            let attempt = save(step.save.as_ref().expect("save"));
            told(
                KioskMachine::check_save(before, &attempt, &who),
                step,
                label,
            );
            untouched.clone()
        }
        "complete" => {
            let held = before.unwrap_or_else(|| panic!("{label}: nothing to complete"));
            let got = told(
                KioskMachine::complete(
                    held,
                    step.submission_id.as_deref().expect("submissionId"),
                    step.document_ref.as_deref().expect("documentRef"),
                ),
                step,
                label,
            );
            match got {
                None => untouched.clone(),
                Some(t) => {
                    let expected = expected_after(step, before, label);
                    assert_eq!(
                        Some(&t.request),
                        expected.as_ref(),
                        "{label}: another request"
                    );
                    assert_eq!(
                        t.event.map(KioskEventKind::as_str),
                        step.event.as_deref(),
                        "{label}: another audit event"
                    );
                    Some(t.request)
                }
            }
        }
        other => panic!("{label}: the fixture names an action the tests do not know: {other}"),
    };
    assert_eq!(
        before.cloned(),
        untouched,
        "{label}: the request handed in was changed"
    );
    after
}

fn expected_after(step: &Step, before: Option<&KioskRequest>, label: &str) -> Option<KioskRequest> {
    match (&step.after, step.unchanged) {
        (Some(a), false) => Some(request(a)),
        (None, true) => before.cloned(),
        _ => panic!("{label}: the fixture says neither what the request became nor that it stayed"),
    }
}
