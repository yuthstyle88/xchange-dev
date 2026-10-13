//! SCAFFOLD (LG-6.0 #1039 up-front tests): the `ShiftBalances` read port, exactly as design
//! `docs/design/blueprint/ledger-inventory.md` §4.2.1 (PR 1037, `expected` signed per PR 1458).
//! Keep these names and signatures; `xc-cash`'s `PgLedgerShiftBalances` implements the trait.

use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};

/// A shift's id. `xc-cash::domain::posting::ShiftId` is the same alias; `xc-shift` cannot name it,
/// because `xc-cash` depends on `xc-shift` (§4.2.1).
pub type ShiftId = String;

/// One currency of a shift's drawer, derived from the ledger (§4.2, L-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftBalance {
    pub currency: CurrencyCode,
    /// The OPENING posting, or the hand-over's, into DRAWER(shift).
    pub opening: Money,
    /// Σ quantity_signed on DRAWER(shift), signed: negative after an approved NEGATIVE_STOCK sale.
    /// For a drawer ≥ 0 it equals `expected_in(Drawer(shift))`.
    pub expected: MoneyDelta,
    /// Σ positive entries on DRAWER(shift) from postings whose source_type ∉ {EXCHANGE, OPENING}.
    pub moved_in: Money,
    /// Σ |negative| entries on the same set.
    pub moved_out: Money,
    /// The shift's verified CLOSE (or HANDOVER_OUT) count: `cash_count_totals.counted`.
    pub counted: Option<Money>,
    /// The same row's difference.
    pub difference: Option<MoneyDelta>,
}

/// Reads a shift's balances back from the ledger.
pub trait ShiftBalances: Send + Sync {
    /// One row per currency with a DRAWER(shift) account or a count line; ordered THB first, then A–Z.
    fn shift_balances<'a>(
        &'a self,
        shift: &'a ShiftId,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftBalance>>>;
}
