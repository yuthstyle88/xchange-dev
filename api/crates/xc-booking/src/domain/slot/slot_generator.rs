//! `SlotGenerator` (design §5, B2): pure. Turns a branch's slot rule, hours and closures into the slots it should
//! offer.

use chrono::{DateTime, NaiveDate, Utc};
use xc_rate::domain::branch::branch_hours::BranchHours;

use super::branch_closure::BranchClosure;
use super::slot_rule::SlotRule;

/// One slot the generator plans; the service stores it as a `booking_slots` row with the rule's capacity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedSlot {
    /// The branch's local date of `start_at`.
    pub service_date: NaiveDate,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
}

pub struct SlotGenerator;

impl SlotGenerator {
    /// For each date from `from` for `days` days, but never past the rule's `horizon_days` (counting `from`): slots
    /// of `slot_minutes` from the window's open, each ending at or before its close (`BranchHours::window`). A
    /// closure date and a `Closed`/`NotSet` day give none. Ordered by `start_at`.
    pub fn slots_for(
        rule: &SlotRule,
        hours: &BranchHours,
        closures: &[BranchClosure],
        from: NaiveDate,
        days: u32,
    ) -> Vec<PlannedSlot> {
        if rule.slot_minutes <= 0 || rule.horizon_days <= 0 || days == 0 {
            return Vec::new();
        }
        let max_days = days.min(rule.horizon_days as u32);
        let mut slots = Vec::new();
        let slot_duration = chrono::Duration::minutes(rule.slot_minutes as i64);

        for day_offset in 0..max_days {
            let Some(date) = from.checked_add_signed(chrono::Duration::days(day_offset as i64))
            else {
                continue;
            };
            if closures.iter().any(|c| c.closed_on == date) {
                continue;
            }
            let window = hours.window(date);
            if let xc_rate::domain::branch::branch_hours::HoursWindow::Open { open, close } = window
            {
                if close <= open {
                    tracing::warn!(
                        date = %date,
                        ?open,
                        ?close,
                        "overnight hours (close <= open) are not supported in BK-2; skipping day"
                    );
                    continue;
                }
                let mut current = open;
                while current + slot_duration <= close {
                    let next = current + slot_duration;
                    slots.push(PlannedSlot {
                        service_date: date,
                        start_at: current,
                        end_at: next,
                    });
                    current = next;
                }
            }
        }
        slots
    }
}
