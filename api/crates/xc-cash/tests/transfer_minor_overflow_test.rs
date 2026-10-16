//! #2063 (caller migration of `Money::minor`, xc-cash transfers): a line's face value × quantity, and the sum of a
//! currency's lines, are computed in checked `i64` hundredths. A total that does not fit is refused as an error; it is
//! never a wrapped number and never a panic.
//!
//! Written before the implementation by the ticket author claude/d7aa64. Do not edit, delete or weaken these tests;
//! add your own in another file.
//!
//! **Why these totals can overflow.** `face_value` is a stored denomination (`denominations.face_value`, read through
//! `Money::parse`, so at most 100,000,000 units = 10^10 hundredths); `qty_sent` / `qty_received` is typed by staff as a
//! whole number up to `i32::MAX` (`whole_number`, `http/v3/commands/cash.rs`). 10^10 × 10^9 = 10^19 > `i64::MAX`
//! (≈ 9.22 × 10^18). Today the product is `face_value.minor() * (qty as i64)`: it panics with overflow checks on
//! (debug, tests) and wraps silently in release (no `overflow-checks` in `[profile.release]`).
//!
//! **Signature-neutral on purpose.** The fix has to turn `TransferEnds::on_dispatch` / `on_receive` into
//! `Result<Vec<PostingRequest>, TransferError>`, `TransferPolicy::is_within_limit` into `Result<bool, TransferError>`,
//! and `TransferLine::sent_minor` / `received_minor` into `Result<i64, _>`. The [`Answer`] trait reads both the old and
//! the new return type, so this file compiles before and after and goes red by *behaviour*, not by a compile error.
//!
//! What is pinned:
//! - an overflowing product or sum → `Err(TransferError::Validation(_))` from `on_dispatch`, `on_receive` and
//!   `is_within_limit` (it reaches the client as `AppError::Domain` → `validationFailed`, 400, through
//!   `from_cash_error`); `Err(_)` (any error type) from `sent_minor` / `received_minor`;
//! - no panic on any of these;
//! - normal totals are unchanged, and the largest product that fits in `i64` is still accepted (the check is
//!   `checked_mul` / `checked_add`, not a tighter cap).

use std::collections::HashSet;
use std::fmt::Debug;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use xc_cash::domain::posting::{CashPlace, PostingRequest};
use xc_cash::domain::transfer::cash_transfer::{
    CashTransfer, TransferError, TransferKind, TransferLine, TransferStatus,
};
use xc_cash::domain::transfer::ends::TransferEnds;
use xc_cash::domain::transfer::kinds::TransferKinds;
use xc_cash::domain::transfer::transfer_policy::TransferPolicy;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

/// The largest face value `Money::parse` accepts: 100,000,000 units = 10^10 hundredths.
const BIG_FACE: &str = "100000000";
/// 10^10 × 10^9 = 10^19 hundredths: past `i64::MAX`.
const OVERFLOWING_QTY: i32 = 1_000_000_000;
/// 10^10 × 922,337,203 = 9,223,372,030,000,000,000 ≤ `i64::MAX` (9,223,372,036,854,775,807): the largest that fits.
const LARGEST_FITTING_QTY: i32 = 922_337_203;
/// Two lines of 10^10 × 5 × 10^8 = 5 × 10^18 each: each fits, their sum (10^19) does not.
const HALF_QTY: i32 = 500_000_000;

// ── reading the old and the new return types ──────────────────────────────

/// What a call answered.
#[derive(Debug)]
enum Got<T> {
    Value(T),
    /// Refused: the error's `Debug`, and whether it is `TransferError::Validation`.
    Refused {
        error: String,
        validation: bool,
    },
}

trait Answer {
    type Value;
    fn got(self) -> Got<Self::Value>;
}

fn lift<T>(r: Result<T, TransferError>) -> Got<T> {
    match r {
        Ok(v) => Got::Value(v),
        Err(e) => Got::Refused {
            error: format!("{e:?}"),
            validation: matches!(e, TransferError::Validation(_)),
        },
    }
}

impl Answer for Vec<PostingRequest> {
    type Value = Vec<PostingRequest>;
    fn got(self) -> Got<Self::Value> {
        Got::Value(self)
    }
}

impl Answer for Result<Vec<PostingRequest>, TransferError> {
    type Value = Vec<PostingRequest>;
    fn got(self) -> Got<Self::Value> {
        lift(self)
    }
}

impl Answer for bool {
    type Value = bool;
    fn got(self) -> Got<bool> {
        Got::Value(self)
    }
}

impl Answer for Result<bool, TransferError> {
    type Value = bool;
    fn got(self) -> Got<bool> {
        lift(self)
    }
}

impl Answer for i64 {
    type Value = i64;
    fn got(self) -> Got<i64> {
        Got::Value(self)
    }
}

/// `sent_minor` / `received_minor` may return any error type.
impl<E: Debug> Answer for Result<i64, E> {
    type Value = i64;
    fn got(self) -> Got<i64> {
        match self {
            Ok(v) => Got::Value(v),
            Err(e) => Got::Refused {
                error: format!("{e:?}"),
                validation: false,
            },
        }
    }
}

/// Runs `f`, failing the test with a clear message if it panics (today's debug behaviour on overflow).
fn answer<A: Answer>(what: &str, f: impl FnOnce() -> A) -> Got<A::Value> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(a) => a.got(),
        Err(_) => panic!(
            "{what}: panicked on an overflowing total; it must be refused with an error instead"
        ),
    }
}

/// Refused with `TransferError::Validation`.
fn assert_refused_validation<T: Debug>(what: &str, got: Got<T>) {
    match got {
        Got::Refused {
            validation: true, ..
        } => {}
        Got::Refused {
            error,
            validation: false,
        } => panic!(
            "{what}: an overflowing total must be Err(TransferError::Validation(_)), got Err({error})"
        ),
        other => panic!(
            "{what}: an overflowing total must be Err(TransferError::Validation(_)), got {other:?}"
        ),
    }
}

/// Refused with any error.
fn assert_refused<T: Debug>(what: &str, got: Got<T>) {
    match got {
        Got::Refused { .. } => {}
        other => panic!("{what}: an overflowing total must be Err(_), got {other:?}"),
    }
}

fn value<T: Debug>(what: &str, got: Got<T>) -> T {
    match got {
        Got::Value(v) => v,
        other => panic!("{what}: a total that fits must be answered, got {other:?}"),
    }
}

// ── fixtures ───────────────────────────────────────────────────────────────

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn eur() -> CurrencyCode {
    CurrencyCode::parse("EUR").unwrap()
}

fn line(
    currency: CurrencyCode,
    denomination_id: i64,
    face: &str,
    sent: i32,
    received: Option<i32>,
) -> TransferLine {
    TransferLine {
        currency,
        denomination_id,
        face_value: Money::parse(face).unwrap(),
        qty_sent: sent,
        qty_received: received,
    }
}

/// Every standard kind, with the branches and shifts its `on_dispatch` / `on_receive` need.
fn every_kind() -> Vec<TransferKind> {
    vec![
        TransferKind::Branch,
        TransferKind::DrawerToSafe,
        TransferKind::SafeToDrawer,
        TransferKind::ExternalIn,
        TransferKind::ExternalOut,
    ]
}

fn transfer(kind: TransferKind, lines: Vec<TransferLine>) -> CashTransfer {
    CashTransfer {
        id: "t-overflow".to_owned(),
        kind,
        from_branch: BranchId::new("B1"),
        to_branch: (kind == TransferKind::Branch).then(|| BranchId::new("B2")),
        from_shift: (kind == TransferKind::DrawerToSafe).then(|| "S1".to_owned()),
        to_shift: (kind == TransferKind::SafeToDrawer).then(|| "S2".to_owned()),
        status: TransferStatus::Approved,
        created_by: "creator".to_owned(),
        lines,
    }
}

fn ends(kind: TransferKind) -> Arc<dyn TransferEnds> {
    TransferKinds::standard()
        .get(kind)
        .unwrap_or_else(|| panic!("{kind:?} is registered"))
}

/// (currency, amount with two places) of each request.
fn amounts(reqs: &[PostingRequest]) -> HashSet<(String, String)> {
    reqs.iter()
        .map(|r| {
            (
                r.currency.as_str().to_owned(),
                format!("{:.2}", r.quantity.value()),
            )
        })
        .collect()
}

fn policy(limit: &str) -> TransferPolicy {
    TransferPolicy::new(
        "v1".to_owned(),
        [(usd(), Money::parse(limit).unwrap())]
            .into_iter()
            .collect(),
    )
}

// ── on_dispatch / on_receive, every kind (kinds/branch.rs, external.rs, safe.rs) ───

#[test]
fn a_dispatch_whose_line_face_value_times_qty_sent_overflows_is_refused_by_every_kind() {
    for kind in every_kind() {
        let t = transfer(kind, vec![line(usd(), 1, BIG_FACE, OVERFLOWING_QTY, None)]);
        let what = format!("{kind:?} on_dispatch, 10^10 × 10^9 hundredths");
        assert_refused_validation(&what, answer(&what, || ends(kind).on_dispatch(&t)));
    }
}

#[test]
fn a_dispatch_whose_currency_sum_overflows_is_refused_by_every_kind_though_each_line_fits() {
    for kind in every_kind() {
        let t = transfer(
            kind,
            vec![
                line(usd(), 1, BIG_FACE, HALF_QTY, None),
                line(usd(), 2, BIG_FACE, HALF_QTY, None),
            ],
        );
        let what = format!("{kind:?} on_dispatch, 5×10^18 + 5×10^18 hundredths");
        assert_refused_validation(&what, answer(&what, || ends(kind).on_dispatch(&t)));
    }
}

#[test]
fn a_receipt_whose_line_face_value_times_qty_received_overflows_is_refused_by_every_kind() {
    for kind in every_kind() {
        let t = transfer(
            kind,
            vec![line(usd(), 1, BIG_FACE, 1, Some(OVERFLOWING_QTY))],
        );
        let what = format!("{kind:?} on_receive, 10^10 × 10^9 hundredths");
        assert_refused_validation(&what, answer(&what, || ends(kind).on_receive(&t)));
    }
}

#[test]
fn a_receipt_whose_currency_sum_overflows_is_refused_by_every_kind_though_each_line_fits() {
    for kind in every_kind() {
        let t = transfer(
            kind,
            vec![
                line(usd(), 1, BIG_FACE, 1, Some(HALF_QTY)),
                line(usd(), 2, BIG_FACE, 1, Some(HALF_QTY)),
            ],
        );
        let what = format!("{kind:?} on_receive, 5×10^18 + 5×10^18 hundredths");
        assert_refused_validation(&what, answer(&what, || ends(kind).on_receive(&t)));
    }
}

#[test]
fn normal_dispatch_and_receipt_totals_are_unchanged_by_every_kind() {
    for kind in every_kind() {
        let t = transfer(
            kind,
            vec![
                line(usd(), 1, "100", 3, Some(2)),
                line(usd(), 2, "20", 4, Some(4)),
                line(eur(), 7, "50", 2, Some(1)),
            ],
        );
        let what = format!("{kind:?} on_dispatch, normal");
        let sent = value(&what, answer(&what, || ends(kind).on_dispatch(&t)));
        assert_eq!(
            amounts(&sent),
            HashSet::from([
                ("USD".to_owned(), "380.00".to_owned()),
                ("EUR".to_owned(), "100.00".to_owned()),
            ]),
            "{what}"
        );
        let what = format!("{kind:?} on_receive, normal");
        let received = value(&what, answer(&what, || ends(kind).on_receive(&t)));
        assert_eq!(
            amounts(&received),
            HashSet::from([
                ("USD".to_owned(), "280.00".to_owned()),
                ("EUR".to_owned(), "50.00".to_owned()),
            ]),
            "{what}"
        );
    }
}

#[test]
fn the_largest_line_total_that_fits_in_i64_hundredths_is_still_posted_by_every_kind() {
    for kind in every_kind() {
        let t = transfer(
            kind,
            vec![line(
                usd(),
                1,
                BIG_FACE,
                LARGEST_FITTING_QTY,
                Some(LARGEST_FITTING_QTY),
            )],
        );
        // 922,337,203 × 100,000,000 units
        let expected = HashSet::from([("USD".to_owned(), "92233720300000000.00".to_owned())]);
        let what = format!("{kind:?} on_dispatch at the i64 edge");
        let sent = value(&what, answer(&what, || ends(kind).on_dispatch(&t)));
        assert_eq!(amounts(&sent), expected, "{what}");
        let what = format!("{kind:?} on_receive at the i64 edge");
        let received = value(&what, answer(&what, || ends(kind).on_receive(&t)));
        assert_eq!(amounts(&received), expected, "{what}");
    }
}

#[test]
fn the_dispatch_and_receipt_places_are_unchanged() {
    // keep-pin: the fix changes only the arithmetic, not who moves what where
    let t = transfer(
        TransferKind::Branch,
        vec![line(usd(), 1, "100", 3, Some(3))],
    );
    let sent = value(
        "branch dispatch",
        answer("branch dispatch", || {
            ends(TransferKind::Branch).on_dispatch(&t)
        }),
    );
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].from, CashPlace::Safe(BranchId::new("B1")));
    assert_eq!(sent[0].to, CashPlace::InTransit("t-overflow".to_owned()));
    let received = value(
        "branch receipt",
        answer("branch receipt", || {
            ends(TransferKind::Branch).on_receive(&t)
        }),
    );
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].from,
        CashPlace::InTransit("t-overflow".to_owned())
    );
    assert_eq!(received[0].to, CashPlace::Safe(BranchId::new("B2")));
}

// ── TransferPolicy::is_within_limit (transfer_policy.rs) ──────────────────

#[test]
fn a_limit_check_whose_line_total_overflows_is_refused_not_read_as_within_the_limit() {
    // Today, in release, 10^19 wraps to -8,446,744,073,709,551,616 hundredths, which is ≤ any limit: the transfer
    // would be "within the limit" and a branch manager could approve it.
    let lines = vec![line(usd(), 1, BIG_FACE, OVERFLOWING_QTY, None)];
    let p = policy("300");
    let what = "is_within_limit, 10^10 × 10^9 hundredths";
    assert_refused_validation(what, answer(what, || p.is_within_limit(&lines)));
}

#[test]
fn a_limit_check_whose_currency_sum_overflows_is_refused_though_each_line_fits() {
    let lines = vec![
        line(usd(), 1, BIG_FACE, HALF_QTY, None),
        line(usd(), 2, BIG_FACE, HALF_QTY, None),
    ];
    let p = policy("300");
    let what = "is_within_limit, 5×10^18 + 5×10^18 hundredths";
    assert_refused_validation(what, answer(what, || p.is_within_limit(&lines)));
}

#[test]
fn normal_limit_checks_are_unchanged() {
    let p = policy("300");
    let within = vec![line(usd(), 1, "50", 6, None)];
    let above = vec![line(usd(), 1, "50", 7, None)];
    let other_currency = vec![line(eur(), 1, "50", 1, None)];
    let none_sent = vec![line(usd(), 1, "50", 0, None)];
    assert!(value(
        "300 of 300",
        answer("300 of 300", || p.is_within_limit(&within))
    ));
    assert!(!value(
        "350 of 300",
        answer("350 of 300", || p.is_within_limit(&above))
    ));
    assert!(!value(
        "no EUR limit",
        answer("no EUR limit", || { p.is_within_limit(&other_currency) })
    ));
    assert!(!value(
        "nothing sent",
        answer("nothing sent", || { p.is_within_limit(&none_sent) })
    ));
    let edge = vec![line(usd(), 1, BIG_FACE, LARGEST_FITTING_QTY, None)];
    assert!(!value(
        "the i64 edge is above 300",
        answer("edge", || { p.is_within_limit(&edge) })
    ));
}

// ── TransferLine::sent_minor / received_minor (cash_transfer.rs) ──────────

#[test]
fn sent_and_received_minor_refuse_an_overflowing_line() {
    let l = line(usd(), 1, BIG_FACE, OVERFLOWING_QTY, Some(OVERFLOWING_QTY));
    assert_refused("sent_minor", answer("sent_minor", || l.sent_minor()));
    assert_refused(
        "received_minor",
        answer("received_minor", || l.received_minor()),
    );
}

#[test]
fn sent_and_received_minor_are_unchanged_for_a_line_that_fits() {
    let l = line(usd(), 1, "100", 3, Some(2));
    assert_eq!(
        value("sent_minor", answer("sent_minor", || l.sent_minor())),
        30_000
    );
    assert_eq!(
        value(
            "received_minor",
            answer("received_minor", || l.received_minor())
        ),
        20_000
    );
    let not_counted = line(usd(), 1, "100", 3, None);
    assert_eq!(
        value(
            "received_minor, not counted",
            answer("received_minor", || { not_counted.received_minor() })
        ),
        0
    );
    let edge = line(usd(), 1, BIG_FACE, LARGEST_FITTING_QTY, None);
    assert_eq!(
        value(
            "sent_minor at the edge",
            answer("sent_minor", || edge.sent_minor())
        ),
        9_223_372_030_000_000_000
    );
}
