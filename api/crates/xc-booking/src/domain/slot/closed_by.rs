//! Why a slot is `CLOSED` (design §4.6, #5392): the `booking_slots.closed_by` values. NULL in the column means the
//! slot is open, or was closed before M3 and the reason is unknown.

/// Who closed a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosedBy {
    /// An ADMIN's `slotClose`; never reopened automatically.
    Slot,
    /// The generator, because the slot's date is a branch closure; reopened when that closure is removed.
    Closure,
    /// The generator, after a rule, hours or horizon change; never reopened automatically.
    Rule,
}

impl ClosedBy {
    /// The stored value.
    pub fn as_str(self) -> &'static str {
        match self {
            ClosedBy::Slot => "SLOT",
            ClosedBy::Closure => "CLOSURE",
            ClosedBy::Rule => "RULE",
        }
    }
}
