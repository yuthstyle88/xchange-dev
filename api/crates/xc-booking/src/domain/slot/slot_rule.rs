//! `SlotRule` (design §5, Q-B1, `booking_slot_rules`): a branch's slot length, capacity and horizon, entered by
//! ADMIN with no default (B11: no number is in code).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotRule {
    /// 5–240 (the table's CHECK).
    pub slot_minutes: i32,
    /// > 0: the capacity each generated slot gets.
    pub capacity: i32,
    /// 1–90: how many days, counting the first, the generator opens.
    pub horizon_days: i32,
}
