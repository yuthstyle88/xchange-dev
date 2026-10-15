//! Written before the implementation (ticket T8a). Do not edit, delete or weaken.
//!
//! Specification of the part of saving an exchange that needs no database
//! (design §3 rows "Amount", "Principal", "BUY", "SELL", "Payment", "Document ref"):
//!
//! - `TransactionForm::parse` — the form of an exchange judged field by field, in
//!   the old server's order and with its sentences, and the three baht figures;
//! - `receipt_html` — the receipt, byte for byte what the old server wrote;
//! - `DocumentRef` — the human-facing number of a saved exchange.
//!
//! The first two replay `tests/fixtures/v2_save_form_cases.json`: 119 forms sent
//! to the v2 engine by `api/tools/v2-fixtures/gen_save_form.mjs`, with what it
//! answered. `tests/fixtures/receipt_template.html` is the fixed markup of the
//! receipt with five `{{…}}` places; the fixture holds what v2 put in them.
//!
//! No database is used here.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use serde::Deserialize;
use xc_kernel::money::{Money, Rate, Side};

use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_exchange::domain::exchange::entities::receipt::receipt_html;
use xc_exchange::domain::exchange::value_objects::document_ref::DocumentRef;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_kernel::mode::Mode;

/// Ticket #670 (TX-2, D-4): the counter key names the organisation. Updated by the ticket author
/// (claude/f03345fe) on the lane controller's ruling of 2026-10-10; OR-1's first organisation.
const ORG_1: uuid::Uuid = uuid::Uuid::from_u128(1);

// The eleven sentences v2 answered these forms with (fixture key `sentences`).
const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
const BAD_DATE_FORMAT: &str = "รูปแบบวันที่ไม่ถูกต้อง";
const BAD_DATE: &str = "วันที่ไม่ถูกต้อง";
const BAD_MODE: &str = "กรุณาเลือกประเภทรายการ";
const BAD_NUMBER: &str = "จำนวนเงินหรือเรทไม่ถูกต้อง";
const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";
const INCOMPLETE: &str = "ข้อมูลรายการไม่ครบ";
const NO_PAYMENT_REF: &str = "ต้องระบุเลขอ้างอิงการโอน";
const TOO_LARGE: &str = "ยอดเงินสูงเกินขอบเขตระบบ";
const FEE_ABOVE_PRINCIPAL: &str = "ค่าบริการสูงกว่ายอดซื้อ";
const FILE_MISSING: &str = "แนบเอกสารไม่ครบ";

const TEMPLATE: &str = include_str!("fixtures/receipt_template.html");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    passport_key: String,
    slip_key: String,
    sentences: Vec<String>,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    form: Form,
    /// What v2 answered: `None` when it saved the form.
    v2_error: Option<String>,
    saved: Option<Saved>,
}

/// The form as the screen sent it. Numbers are text here; the screens send JSON numbers.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Form {
    name: String,
    passport_no: String,
    nationality: String,
    birth_date: String,
    passport_expiry: String,
    mode: String,
    submission_id: String,
    customer_id: String,
    r#type: String,
    currency: String,
    amount: String,
    rate: String,
    fee: String,
    bank_fee: String,
    method: String,
    r#ref: String,
    account: String,
    staff: String,
    booking: String,
    notes: String,
    value_date: String,
    paid_at: String,
    payment_confirmed: bool,
    identity_confirmed: bool,
    passport: Option<String>,
    slip: Option<String>,
    reference_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    document_ref: String,
    branch_name: String,
    principal: String,
    received: String,
    paid: String,
    stored: Stored,
    receipt: ReceiptParts,
}

/// What v2 stored of the form, after trimming.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    r#type: String,
    currency: String,
    amount: String,
    rate: String,
    fee: String,
    bank_fee: String,
    method: String,
    r#ref: String,
    account: String,
    staff: String,
    booking: String,
    notes: String,
}

/// What v2 put in the five places of the receipt template.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReceiptParts {
    title: String,
    demo_banner: String,
    currency: String,
    rows: String,
    total: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/v2_save_form_cases.json"))
        .expect("tests/fixtures/v2_save_form_cases.json parses")
}

fn command(form: &Form) -> SaveTransaction {
    SaveTransaction {
        customer: ReserveCustomer {
            name: form.name.clone(),
            passport_no: form.passport_no.clone(),
            nationality: form.nationality.clone(),
            birth_date: form.birth_date.clone(),
            passport_expiry: form.passport_expiry.clone(),
            mode: form.mode.clone(),
        },
        submission_id: form.submission_id.clone(),
        customer_id: form.customer_id.clone(),
        side: form.r#type.clone(),
        currency: form.currency.clone(),
        amount: form.amount.clone(),
        rate: form.rate.clone(),
        fee: form.fee.clone(),
        bank_fee: form.bank_fee.clone(),
        method: form.method.clone(),
        payment_ref: form.r#ref.clone(),
        account: form.account.clone(),
        staff: form.staff.clone(),
        booking: form.booking.clone(),
        notes: form.notes.clone(),
        value_date: form.value_date.clone(),
        paid_at: form.paid_at.clone(),
        payment_confirmed: form.payment_confirmed,
        identity_confirmed: form.identity_confirmed,
        passport: form.passport.clone(),
        slip: form.slip.clone(),
        reference_rate: form.reference_rate.clone(),
    }
}

/// The sentence of a refusal; panics on anything else.
fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

/// A form v2 saves: a purchase of 100 USD at 32.1 for cash.
fn good() -> SaveTransaction {
    let fixture = fixture();
    let case = fixture
        .cases
        .iter()
        .find(|c| c.name == "buy for cash")
        .expect("the first case of the fixture");
    command(&case.form)
}

fn parsed(cmd: &SaveTransaction) -> TransactionForm {
    TransactionForm::parse(cmd).expect("a form v2 saves")
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 time")
        .with_timezone(&Utc)
}

// ── the fixture ─────────────────────────────────────────────────────────────

#[test]
fn the_fixture_is_the_one_this_file_was_written_for() {
    let fixture = fixture();
    assert_eq!(fixture.cases.len(), 119);
    assert_eq!(
        fixture.cases.iter().filter(|c| c.saved.is_some()).count(),
        37
    );
    assert_eq!(
        fixture
            .cases
            .iter()
            .filter(|c| c.v2_error.is_some())
            .count(),
        82
    );
    let mut known = vec![
        BAD_TEXT,
        BAD_DATE_FORMAT,
        BAD_DATE,
        BAD_MODE,
        BAD_NUMBER,
        TOO_MANY_DECIMALS,
        INCOMPLETE,
        NO_PAYMENT_REF,
        TOO_LARGE,
        FEE_ABOVE_PRINCIPAL,
        FILE_MISSING,
    ];
    known.sort_unstable();
    assert_eq!(
        fixture.sentences, known,
        "the sentences v2 used are the eleven constants"
    );
    for place in [
        "{{TITLE}}",
        "{{DEMO_BANNER}}",
        "{{CURRENCY}}",
        "{{ROWS}}",
        "{{TOTAL}}",
    ] {
        assert_eq!(
            TEMPLATE.matches(place).count(),
            1,
            "{place} appears once in the template"
        );
    }
}

// ── TransactionForm::parse ──────────────────────────────────────────────────

#[test]
fn every_form_v2_refused_is_refused_with_the_same_sentence() {
    let fixture = fixture();
    let mut wrong = Vec::new();
    for case in fixture.cases.iter().filter(|c| c.v2_error.is_some()) {
        let want = case.v2_error.as_deref().unwrap();
        match TransactionForm::parse(&command(&case.form)) {
            Err(AppError::Domain(sentence)) if sentence == want => {}
            other => wrong.push(format!(
                "'{}': v2 answered {want:?}, parse answered {other:?}",
                case.name
            )),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} difference(s) from v2:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn every_form_v2_saved_parses_to_what_v2_stored() {
    let fixture = fixture();
    let mut wrong = Vec::new();
    for case in fixture.cases.iter().filter(|c| c.saved.is_some()) {
        let saved = case.saved.as_ref().unwrap();
        let form = match TransactionForm::parse(&command(&case.form)) {
            Ok(form) => form,
            Err(e) => {
                wrong.push(format!(
                    "'{}': v2 saved it, parse answered {e:?}",
                    case.name
                ));
                continue;
            }
        };
        let got = [
            (
                "side",
                match form.side {
                    Side::Buy => "BUY",
                    Side::Sell => "SELL",
                }
                .to_owned(),
            ),
            ("currency", form.currency.clone()),
            ("amount", form.amount.to_string()),
            ("rate", form.rate.to_string()),
            ("fee", form.fee.to_string()),
            ("bank fee", form.bank_fee.to_string()),
            ("method", form.method.as_str().to_owned()),
            ("payment ref", form.payment_ref.clone()),
            ("account", form.account.clone()),
            ("staff", form.staff.clone()),
            ("booking", form.booking.clone()),
            ("notes", form.notes.clone()),
            ("principal", form.quote.principal.to_string()),
            ("received", form.quote.received.to_string()),
            ("paid", form.quote.paid.to_string()),
            ("submission id", form.submission_id.clone()),
            ("customer id", form.customer_id.clone()),
            ("customer name", form.customer.name.clone()),
            ("value date", form.value_date.to_string()),
        ];
        let st = &saved.stored;
        let want = [
            &st.r#type,
            &st.currency,
            &st.amount,
            &st.rate,
            &st.fee,
            &st.bank_fee,
            &st.method,
            &st.r#ref,
            &st.account,
            &st.staff,
            &st.booking,
            &st.notes,
            &saved.principal,
            &saved.received,
            &saved.paid,
        ];
        for ((what, got), want) in got.iter().zip(want) {
            if got != want {
                wrong.push(format!(
                    "'{}': {what} is {got:?}, v2 stored {want:?}",
                    case.name
                ));
            }
        }
        // The rest of `got` is the form's own text, trimmed.
        for ((what, got), want) in got[15..].iter().zip([
            case.form.submission_id.trim(),
            case.form.customer_id.trim(),
            case.form.name.trim(),
            case.form.value_date.trim(),
        ]) {
            if got != want {
                wrong.push(format!(
                    "'{}': {what} is {got:?}, the trimmed form has {want:?}",
                    case.name
                ));
            }
        }
        if form.paid_at != instant(case.form.paid_at.trim()) {
            wrong.push(format!("'{}': paid_at is {:?}", case.name, form.paid_at));
        }
        if Some(&form.passport) != case.form.passport.as_ref() || form.slip != case.form.slip {
            wrong.push(format!(
                "'{}': files are {:?} / {:?}",
                case.name, form.passport, form.slip
            ));
        }
        let want_reference = case
            .form
            .reference_rate
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| Rate::parse(s).unwrap());
        if form.reference_rate != want_reference {
            wrong.push(format!(
                "'{}': reference rate is {:?}",
                case.name, form.reference_rate
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} difference(s) from v2:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_parsed_form_holds_typed_values() {
    let fixture = fixture();
    let mut cmd = good();
    cmd.side = "SELL".to_owned();
    cmd.method = "SCB Transfer".to_owned();
    cmd.payment_ref = " TRX-1 ".to_owned();
    cmd.slip = Some(fixture.slip_key.clone());
    cmd.fee = "25.5".to_owned();
    cmd.reference_rate = Some("32.25".to_owned());
    cmd.customer.mode = "DEMO".to_owned();
    cmd.customer.passport_no = " aa1234567 ".to_owned();
    cmd.paid_at = "2026-10-06T12:30:00+07:00".to_owned();

    let form = parsed(&cmd);

    assert_eq!(form.side, Side::Sell);
    assert_eq!(form.method, PaymentMethod::ScbTransfer);
    assert_eq!(form.payment_ref, "TRX-1");
    assert_eq!(form.amount, Money::parse("100").unwrap());
    assert_eq!(form.rate, Rate::parse("32.1").unwrap());
    assert_eq!(form.fee, Money::parse("25.5").unwrap());
    assert_eq!(form.bank_fee, Money::ZERO);
    assert_eq!(form.reference_rate, Some(Rate::parse("32.25").unwrap()));
    assert_eq!(form.quote.principal, Money::parse("3210").unwrap());
    assert_eq!(form.quote.received, Money::parse("3235.5").unwrap());
    assert_eq!(form.quote.paid, Money::ZERO);
    assert_eq!(form.customer.mode, Mode::Demo);
    assert_eq!(
        form.customer.passport_no, "AA1234567",
        "the customer's fields are judged by CustomerDetails::parse"
    );
    assert_eq!(
        form.value_date,
        NaiveDate::from_ymd_opt(2026, 10, 6).unwrap()
    );
    assert_eq!(
        form.paid_at,
        Utc.with_ymd_and_hms(2026, 10, 6, 5, 30, 0).unwrap(),
        "an offset is the same instant"
    );
    assert_eq!(form.passport, fixture.passport_key);
    assert_eq!(form.slip, Some(fixture.slip_key));
}

#[test]
fn the_six_payment_methods_print_as_the_screens_send_them() {
    let all = [
        (PaymentMethod::Cash, "Cash", true),
        (PaymentMethod::ScbTransfer, "SCB Transfer", false),
        (PaymentMethod::BankTransfer, "Bank Transfer", false),
        (PaymentMethod::CreditCard, "Credit Card", false),
        (PaymentMethod::Alipay, "Alipay", false),
        (PaymentMethod::PromptPay, "PromptPay", false),
    ];
    for (method, text, cash) in all {
        assert_eq!(method.as_str(), text);
        assert_eq!(method.is_cash(), cash, "{text}");
        let mut cmd = good();
        cmd.method = text.to_owned();
        cmd.payment_ref = "R-1".to_owned();
        cmd.slip = Some("22222222-2222-4222-8222-222222222222".to_owned());
        assert_eq!(parsed(&cmd).method, method);
    }
}

#[test]
fn a_payment_time_must_be_rfc_3339() {
    // The screens send `new Date().toISOString()`. v2 took anything JavaScript's
    // `Date.parse` accepts; v3 takes RFC 3339 only and calls the rest incomplete.
    for ok in [
        "2026-10-06T05:30:00.000Z",
        "2026-10-06T05:30:00Z",
        "2026-10-06T12:30:00+07:00",
        "2026-10-06T05:30:00.123456Z",
    ] {
        let mut cmd = good();
        cmd.paid_at = ok.to_owned();
        assert!(TransactionForm::parse(&cmd).is_ok(), "{ok}");
    }
    for bad in [
        "2026-10-06",
        "2026-10-06T05:30:00",
        "06/10/2026 12:30",
        "1791264600000",
        "2026-13-06T05:30:00Z",
    ] {
        let mut cmd = good();
        cmd.paid_at = bad.to_owned();
        assert_eq!(refusal(TransactionForm::parse(&cmd)), INCOMPLETE, "{bad}");
    }
}

#[test]
fn a_cash_exchange_needs_no_reference_and_no_slip_but_keeps_them_when_sent() {
    let mut cmd = good();
    assert_eq!(parsed(&cmd).slip, None);
    assert_eq!(parsed(&cmd).payment_ref, "");
    cmd.slip = Some("22222222-2222-4222-8222-222222222222".to_owned());
    cmd.payment_ref = "R-1".to_owned();
    let form = parsed(&cmd);
    assert_eq!(
        form.slip.as_deref(),
        Some("22222222-2222-4222-8222-222222222222")
    );
    assert_eq!(form.payment_ref, "R-1");
}

// ── receipt_html ────────────────────────────────────────────────────────────

fn expected_receipt(parts: &ReceiptParts) -> String {
    TEMPLATE
        .replacen("{{TITLE}}", &parts.title, 1)
        .replacen("{{DEMO_BANNER}}", &parts.demo_banner, 1)
        .replacen("{{CURRENCY}}", &parts.currency, 1)
        .replacen("{{ROWS}}", &parts.rows, 1)
        .replacen("{{TOTAL}}", &parts.total, 1)
}

#[test]
fn every_receipt_is_byte_for_byte_the_one_v2_wrote() {
    let fixture = fixture();
    let mut wrong = Vec::new();
    for case in fixture.cases.iter().filter(|c| c.saved.is_some()) {
        let saved = case.saved.as_ref().unwrap();
        let Ok(form) = TransactionForm::parse(&command(&case.form)) else {
            wrong.push(format!("'{}': the form does not parse", case.name));
            continue;
        };
        let got = receipt_html(&form, &saved.document_ref, &saved.branch_name);
        let want = expected_receipt(&saved.receipt);
        if got != want {
            let at = got
                .bytes()
                .zip(want.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(want.len()));
            let show = |s: &str| {
                let from = (0..=at.saturating_sub(60))
                    .rev()
                    .find(|i| s.is_char_boundary(*i))
                    .unwrap_or(0);
                let to = (at + 60..=s.len())
                    .find(|i| s.is_char_boundary(*i))
                    .unwrap_or(s.len());
                s.get(from..to).unwrap_or("").to_owned()
            };
            wrong.push(format!(
                "'{}': differs at byte {at} (lengths {} and {})\n   got: …{}…\n  want: …{}…",
                case.name,
                got.len(),
                want.len(),
                show(&got),
                show(&want)
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} receipt(s) differ from v2:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn the_receipt_shows_bangkok_time_and_groups_the_total_by_thousands() {
    let mut cmd = good();
    cmd.side = "SELL".to_owned();
    cmd.amount = "1234567.89".to_owned();
    cmd.rate = "1".to_owned();
    cmd.paid_at = "2026-12-31T17:00:00Z".to_owned();
    let html = receipt_html(&parsed(&cmd), "XC-20270101-000001", "XC-B");
    assert!(
        html.contains("<dt>Time</dt><dd>01/01/2027 00:00</dd>"),
        "17:00 UTC is midnight in Bangkok, the next day"
    );
    assert!(html.contains("<dt>THB Total</dt><dd>1,234,567.89 THB</dd>"));
    assert!(
        html.contains("<dt>Amount</dt><dd>1234567.89</dd>"),
        "only the total is grouped"
    );

    cmd.amount = "0.5".to_owned();
    let html = receipt_html(&parsed(&cmd), "XC-1", "XC-B");
    assert!(html.contains("<dt>THB Total</dt><dd>0.50 THB</dd>"));
    cmd.amount = "999.99".to_owned();
    assert!(receipt_html(&parsed(&cmd), "XC-1", "XC-B").contains("<dd>999.99 THB</dd>"));
    cmd.amount = "1000".to_owned();
    assert!(receipt_html(&parsed(&cmd), "XC-1", "XC-B").contains("<dd>1,000.00 THB</dd>"));
}

#[test]
fn the_receipt_escapes_the_five_characters_in_every_value() {
    let mut cmd = good();
    cmd.customer.name = "a&b<c>d\"e'f".to_owned();
    cmd.staff = "{{TOTAL}}".to_owned();
    let html = receipt_html(&parsed(&cmd), "R&D<1>", "B'1\"");
    assert!(html.contains("<dt>Customer</dt><dd>a&amp;b&lt;c&gt;d&quot;e&#39;f</dd>"));
    assert!(html.contains("<title>R&amp;D&lt;1&gt;</title>"));
    assert!(html.contains("<dt>Receipt No</dt><dd>R&amp;D&lt;1&gt;</dd>"));
    assert!(html.contains("<dt>Branch</dt><dd>B&#39;1&quot;</dd>"));
    assert!(
        html.contains("<dt>Staff</dt><dd>{{TOTAL}}</dd>"),
        "a value that looks like a place of the template is just a value"
    );
    assert!(html.contains("<dd>3,210.00 THB</dd>"));
    assert!(
        !html.contains("{{TITLE}}")
            && !html.contains("{{ROWS}}")
            && !html.contains("{{DEMO_BANNER}}")
            && !html.contains("{{CURRENCY}}")
    );
}

// ── DocumentRef ─────────────────────────────────────────────────────────────

#[test]
fn a_document_number_carries_the_bangkok_date_and_at_least_six_digits() {
    let noon = Utc.with_ymd_and_hms(2026, 10, 6, 5, 0, 0).unwrap();
    assert_eq!(
        DocumentRef::format(Mode::Live, noon, 1),
        "XC-20261006-000001"
    );
    assert_eq!(
        DocumentRef::format(Mode::Demo, noon, 1),
        "TEST-XC-20261006-000001"
    );
    assert_eq!(
        DocumentRef::format(Mode::Live, noon, 999_999),
        "XC-20261006-999999"
    );
    assert_eq!(
        DocumentRef::format(Mode::Live, noon, 1_000_000),
        "XC-20261006-1000000"
    );
    assert_eq!(
        DocumentRef::counter_key(ORG_1, Mode::Live, noon),
        "document-00000000-0000-0000-0000-000000000001-LIVE-20261006"
    );
    assert_eq!(
        DocumentRef::counter_key(ORG_1, Mode::Demo, noon),
        "document-00000000-0000-0000-0000-000000000001-DEMO-20261006"
    );
}

#[test]
fn the_day_of_a_document_number_changes_at_bangkok_midnight() {
    let before = Utc.with_ymd_and_hms(2026, 12, 31, 16, 59, 59).unwrap();
    let after = Utc.with_ymd_and_hms(2026, 12, 31, 17, 0, 0).unwrap();
    assert_eq!(
        DocumentRef::format(Mode::Live, before, 7),
        "XC-20261231-000007"
    );
    assert_eq!(
        DocumentRef::format(Mode::Live, after, 7),
        "XC-20270101-000007"
    );
    assert_eq!(
        DocumentRef::counter_key(ORG_1, Mode::Live, before),
        "document-00000000-0000-0000-0000-000000000001-LIVE-20261231"
    );
    assert_eq!(
        DocumentRef::counter_key(ORG_1, Mode::Live, after),
        "document-00000000-0000-0000-0000-000000000001-LIVE-20270101"
    );
}

#[test]
fn the_fixtures_document_numbers_have_the_shape_document_ref_prints() {
    let fixture = fixture();
    for case in fixture.cases.iter().filter(|c| c.saved.is_some()) {
        let saved = case.saved.as_ref().unwrap();
        let demo = case.form.mode == "DEMO";
        let rest = saved
            .document_ref
            .strip_prefix(if demo { "TEST-XC-" } else { "XC-" })
            .unwrap_or_else(|| panic!("'{}': {}", case.name, saved.document_ref));
        let (date, n) = rest.split_once('-').expect("date-number");
        let day = NaiveDate::parse_from_str(date, "%Y%m%d").expect("yyyymmdd");
        // Noon in Bangkok on that day.
        let at = Utc.from_utc_datetime(&day.and_hms_opt(5, 0, 0).unwrap());
        let mode = if demo { Mode::Demo } else { Mode::Live };
        assert_eq!(
            DocumentRef::format(mode, at, n.parse().unwrap()),
            saved.document_ref,
            "'{}'",
            case.name
        );
    }
}
