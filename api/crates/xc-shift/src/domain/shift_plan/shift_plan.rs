//! A branch's live shift templates and every time question about them (configurable-shifts.md §2.2).
//!
//! Wall-clock times are in the branch timezone. The anchor `S0` is the start that follows the longest stretch of the
//! 24-hour clock no template covers; ties, or no uncovered stretch, take the earliest such start (T1). A template's
//! instance on business date `d` runs from `d + S0 + startOffset` to `d + S0 + endOffset` (T2, T3).

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;

use super::plan_refusal::PlanRefusal;
use super::shift_instance::ShiftInstance;
use super::shift_template::{ShiftTemplate, ShiftTemplateInput};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftPlan {
    /// Sorted by `position`.
    templates: Vec<ShiftTemplate>,
    tz: Tz,
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).expect("a valid wall-clock time")
}

impl ShiftPlan {
    pub const MAX_TEMPLATES: usize = 12;
    pub const MAX_NAME_CHARS: usize = 40;

    /// T5. Trims names and returns the inputs in the given order, or the first refusal found
    /// (row checks in row order, then `SpanOver24h`).
    pub fn validate(
        inputs: Vec<ShiftTemplateInput>,
    ) -> Result<Vec<ShiftTemplateInput>, PlanRefusal> {
        let _ = inputs;
        todo!("CS-1")
    }

    pub fn new(mut templates: Vec<ShiftTemplate>, tz: Tz) -> Self {
        templates.sort_by_key(|t| t.position);
        Self { templates, tz }
    }

    /// The owner's defaults, 2026-11-04 ("default ตามนี้"). The migration seeds the same rows.
    pub fn defaults() -> Vec<ShiftTemplateInput> {
        vec![
            ShiftTemplateInput {
                id: None,
                name: "A".to_owned(),
                starts_at: hm(10, 0),
                ends_at: hm(19, 0),
                break_window: Some((hm(15, 0), hm(16, 0))),
            },
            ShiftTemplateInput {
                id: None,
                name: "B".to_owned(),
                starts_at: hm(15, 0),
                ends_at: hm(0, 0),
                break_window: Some((hm(18, 0), hm(19, 0))),
            },
            ShiftTemplateInput {
                id: None,
                name: "C".to_owned(),
                starts_at: hm(19, 0),
                ends_at: hm(4, 0),
                break_window: Some((hm(23, 0), hm(0, 0))),
            },
        ]
    }

    pub fn templates(&self) -> &[ShiftTemplate] {
        &self.templates
    }

    pub fn timezone(&self) -> Tz {
        self.tz
    }

    /// T3: `template` on business `date`.
    pub fn instance(&self, template: &ShiftTemplate, date: NaiveDate) -> ShiftInstance {
        let _ = (template, date);
        todo!("CS-1")
    }

    /// T6: the instances whose `[starts_at, ends_at)` holds `now`, for business dates local today and local
    /// yesterday; plan order, and for one template the earlier date first.
    pub fn covering(&self, now: DateTime<Utc>) -> Vec<ShiftInstance> {
        let _ = now;
        todo!("CS-1")
    }

    /// T7: the business date whose service span holds `now`; `None` outside every span.
    pub fn business_date_of(&self, now: DateTime<Utc>) -> Option<NaiveDate> {
        let _ = now;
        todo!("CS-1")
    }

    /// T4: `[date + S0, date + S0 + max endOffset)`; `None` for an empty plan.
    pub fn service_span(&self, date: NaiveDate) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let _ = date;
        todo!("CS-1")
    }

    /// The span holding `now`, else the next span to start (today's or tomorrow's); `None` for an empty plan.
    pub fn span_for(&self, now: DateTime<Utc>) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let _ = now;
        todo!("CS-1")
    }

    /// True while `now` is inside the break of a covering instance of `template_id`.
    pub fn on_break(&self, template_id: &str, now: DateTime<Utc>) -> bool {
        let _ = (template_id, now);
        todo!("CS-1")
    }
}
