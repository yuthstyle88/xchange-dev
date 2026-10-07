//! Written before the implementation (row PM-R, design `docs/design/xchange-v3.md` §5.5.2).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The payment rules of a branch as one pure value: the five channels, which of them may pay
//! baht out (BUY) or take baht in (SELL), and which method name a save or a kiosk request may use.
//!
//! `fixtures/v2_payment_rules_cases.json` is what the 10-06 bundle's `payment-settings.mjs`
//! answered (`api/tools/v2-fixtures/gen_payment_rules.mjs`): its `METHODS`, its `defaults()`,
//! what `validateMethods` accepted or refused for each set of five rows, and what `allowed`
//! answered for each method name and direction on each accepted set.
//!
//! `PaymentRules::from_rows` is the typed half of `validateMethods`: it gets rows whose values
//! are already booleans and texts, so it can only refuse (with `None`) a set that is not the
//! five channels once each, or one that breaks the BUY-channel, direction or one-per-side rule.
//! The sentence of each refusal is the parser's (row PM-API), not this value's.

use serde::Deserialize;
use xc_kernel::money::Side;
use xc_kernel::payment::{ChannelRule, METHODS, PaymentRules};

#[derive(Debug, Clone, Deserialize)]
struct Row {
    id: String,
    enabled: bool,
    buy: bool,
    sell: bool,
    account: String,
    instruction: String,
}

#[derive(Debug, Deserialize)]
struct FromRows {
    label: String,
    rows: Vec<Row>,
    accepted: Option<Vec<Row>>,
    refused: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Answer {
    method: String,
    r#type: String,
    allowed: bool,
}

#[derive(Debug, Deserialize)]
struct Allows {
    label: String,
    rows: Vec<Row>,
    answers: Vec<Answer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    methods: Vec<String>,
    defaults: Vec<Row>,
    from_rows: Vec<FromRows>,
    allows: Vec<Allows>,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/v2_payment_rules_cases.json"))
        .expect("v2_payment_rules_cases.json parses")
}

fn rule(row: &Row) -> ChannelRule {
    ChannelRule {
        id: row.id.clone(),
        enabled: row.enabled,
        buy: row.buy,
        sell: row.sell,
        account: row.account.clone(),
        instruction: row.instruction.clone(),
    }
}

fn rules(rows: &[Row]) -> Vec<ChannelRule> {
    rows.iter().map(rule).collect()
}

fn side(text: &str) -> Side {
    match text {
        "BUY" => Side::Buy,
        "SELL" => Side::Sell,
        other => panic!("the fixture names the direction {other}"),
    }
}

const NO_BUY_HERE: &str = "การจ่ายเงินบาทให้ลูกค้ารองรับเงินสดและโอนธนาคาร";
const ON_WITH_NO_DIRECTION: &str = "ช่องทางที่เปิดต้องเลือกการรับเงินหรือจ่ายเงินอย่างน้อยหนึ่งประเภท";
const ONE_PER_DIRECTION: &str = "ต้องเหลือช่องทางรับเงินและจ่ายเงินบาทอย่างน้อยประเภทละ 1 ช่องทาง";

#[test]
fn the_five_channels_are_v2s_in_v2s_order() {
    let fixture = fixture();
    assert_eq!(
        METHODS.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
        fixture.methods
    );
    assert_eq!(
        METHODS,
        [
            "Cash",
            "Credit Card",
            "Alipay",
            "PromptPay",
            "Bank Transfer"
        ]
    );
}

#[test]
fn the_defaults_are_v2s_row_for_row() {
    let fixture = fixture();
    let defaults = PaymentRules::defaults();
    assert_eq!(defaults.rows().to_vec(), rules(&fixture.defaults));
    // read out, so a reader of this file sees what "the defaults" are
    let on: Vec<(&str, bool, bool, bool)> = defaults
        .rows()
        .iter()
        .map(|r| (r.id.as_str(), r.enabled, r.buy, r.sell))
        .collect();
    assert_eq!(
        on,
        vec![
            ("Cash", true, true, true),
            ("Credit Card", true, false, true),
            ("Alipay", true, false, true),
            ("PromptPay", true, false, true),
            ("Bank Transfer", true, true, true),
        ]
    );
    assert_eq!(
        defaults.rows()[4].instruction,
        "If transfer, call agent · หากโอนเงิน กรุณาติดต่อพนักงาน"
    );
    assert!(
        defaults
            .rows()
            .iter()
            .all(|r| r.account.is_empty() && (r.id == "Bank Transfer" || r.instruction.is_empty()))
    );
}

#[test]
fn from_rows_accepts_what_v2_accepts_and_puts_it_in_v2s_order() {
    let fixture = fixture();
    let mut accepted = 0;
    for case in &fixture.from_rows {
        let Some(expected) = &case.accepted else {
            continue;
        };
        let got = PaymentRules::from_rows(rules(&case.rows))
            .unwrap_or_else(|| panic!("{}: v2 accepted it", case.label));
        assert_eq!(got.rows().to_vec(), rules(expected), "{}", case.label);
        accepted += 1;
    }
    assert_eq!(accepted, 8, "every accepted set of the fixture was tried");
}

#[test]
fn from_rows_refuses_every_set_v2_refuses() {
    let fixture = fixture();
    let mut refused = Vec::new();
    for case in &fixture.from_rows {
        let Some(sentence) = &case.refused else {
            continue;
        };
        assert_eq!(
            PaymentRules::from_rows(rules(&case.rows)),
            None,
            "{}: v2 refused it with {sentence}",
            case.label
        );
        refused.push(sentence.as_str());
    }
    // the three rules this value owns are each among the refusals tried
    for rule in [NO_BUY_HERE, ON_WITH_NO_DIRECTION, ONE_PER_DIRECTION] {
        assert!(refused.contains(&rule), "a case refused with {rule}");
    }
    assert_eq!(refused.len(), 14);
}

/// The notes of the 10-06 bundle: "BUY รองรับเงินสดและโอนธนาคาร" — baht is paid out only in cash
/// or by bank transfer, whether the channel is on or off.
#[test]
fn only_cash_and_bank_transfer_may_pay_baht_out() {
    for id in ["Credit Card", "Alipay", "PromptPay"] {
        for enabled in [true, false] {
            let rows: Vec<ChannelRule> = PaymentRules::defaults()
                .rows()
                .iter()
                .cloned()
                .map(|mut r| {
                    if r.id == id {
                        r.enabled = enabled;
                        r.buy = true;
                    }
                    r
                })
                .collect();
            assert_eq!(
                PaymentRules::from_rows(rows),
                None,
                "BUY on {id} (on: {enabled})"
            );
        }
    }
    for id in ["Cash", "Bank Transfer"] {
        let rows: Vec<ChannelRule> = PaymentRules::defaults()
            .rows()
            .iter()
            .cloned()
            .map(|mut r| {
                if r.id == id {
                    r.sell = false;
                }
                r
            })
            .collect();
        let accepted = PaymentRules::from_rows(rows).expect("BUY only on a cash or transfer row");
        assert!(accepted.allows(id, Side::Buy), "{id} pays baht out");
        assert!(
            !accepted.allows(id, Side::Sell),
            "{id} no longer takes baht in"
        );
    }
}

/// "ต้องเหลืออย่างน้อยหนึ่งช่องทางต่อทิศทาง": switching the last BUY channel, or the last SELL
/// channel, off is refused; a channel that is off does not count, even if it is marked.
#[test]
fn at_least_one_channel_stays_on_for_each_direction() {
    let with = |change: &dyn Fn(&mut ChannelRule)| -> Option<PaymentRules> {
        let rows = PaymentRules::defaults()
            .rows()
            .iter()
            .cloned()
            .map(|mut r| {
                change(&mut r);
                r
            })
            .collect();
        PaymentRules::from_rows(rows)
    };
    assert_eq!(
        with(&|r| {
            if r.id == "Cash" || r.id == "Bank Transfer" {
                r.enabled = false;
            }
        }),
        None,
        "BUY marked only on channels that are off"
    );
    assert_eq!(
        with(&|r| {
            r.enabled = r.id == "Cash";
            if r.id == "Cash" {
                r.sell = false;
            }
        }),
        None,
        "SELL on no channel that is on"
    );
    let last = with(&|r| r.enabled = r.id == "Bank Transfer").expect("one channel on for both");
    assert!(last.allows("Bank Transfer", Side::Buy));
    assert!(last.allows("Bank Transfer", Side::Sell));
    assert!(!last.allows("Cash", Side::Buy));
}

#[test]
fn allows_answers_as_v2_for_every_method_and_direction() {
    let fixture = fixture();
    let mut asked = 0;
    for set in &fixture.allows {
        let rules = PaymentRules::from_rows(rules(&set.rows))
            .unwrap_or_else(|| panic!("{}: the set v2 accepted", set.label));
        for answer in &set.answers {
            assert_eq!(
                rules.allows(&answer.method, side(&answer.r#type)),
                answer.allowed,
                "{}: {:?} for {}",
                set.label,
                answer.method,
                answer.r#type
            );
            asked += 1;
        }
    }
    assert_eq!(asked, 160);
}

/// v2's `canonical`: the old name of the transfer channel is read as `Bank Transfer`; any other
/// spelling is no channel at all.
#[test]
fn scb_transfer_is_read_as_bank_transfer_and_nothing_else_is_renamed() {
    let only_transfer = PaymentRules::from_rows(
        PaymentRules::defaults()
            .rows()
            .iter()
            .cloned()
            .map(|mut r| {
                r.enabled = r.id == "Bank Transfer";
                r
            })
            .collect(),
    )
    .expect("transfer only");
    for side in [Side::Buy, Side::Sell] {
        assert!(only_transfer.allows("SCB Transfer", side), "{side:?}");
        assert!(only_transfer.allows("Bank Transfer", side), "{side:?}");
        for other in ["Cash", "", "bank transfer", "Bank transfer", "SCB transfer"] {
            assert!(!only_transfer.allows(other, side), "{other:?} {side:?}");
        }
    }
}

#[test]
fn rule_finds_each_channel_by_its_name() {
    let defaults = PaymentRules::defaults();
    for (i, id) in METHODS.iter().enumerate() {
        assert_eq!(defaults.rule(id), Some(&defaults.rows()[i]), "{id}");
    }
    assert_eq!(defaults.rule("Cheque"), None);
    assert_eq!(defaults.rule(""), None);
}
