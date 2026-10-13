//! Attributed contribution (D-15; ruling 4 on #1406).

use xc_kernel::money::{Money, MoneyDelta};

use super::unavailable::Unavailable;

/// realized margin − REWARD expense − commission.
///
/// - PRICE cost is not an input: it is already inside the realized margin and is never subtracted again (D-15).
/// - `margin` unavailable ⇒ `Err(AwaitsLedger)`, whatever the commission is (the ledger reason comes first).
/// - otherwise `commission` unavailable ⇒ its reason (`AwaitsBroker` today).
/// - never a zero in place of an unavailable term.
pub fn attributed_contribution(
    margin: Result<MoneyDelta, Unavailable>,
    reward_expense: Money,
    commission: Result<Money, Unavailable>,
) -> Result<MoneyDelta, Unavailable> {
    let margin = margin?;
    let commission = commission?;
    Ok(margin.minus(reward_expense).minus(commission))
}
