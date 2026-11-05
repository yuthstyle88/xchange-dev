//! A branch's live shift templates and every time question about them (configurable-shifts.md §2.2).
//!
//! Wall-clock times are in the branch timezone. The anchor `S0` is the start that follows the longest stretch of the
//! 24-hour clock no template covers; ties, or no uncovered stretch, take the earliest such start (T1). A template's
//! instance on business date `d` runs from `d + S0 + startOffset` to `d + S0 + endOffset` (T2, T3).
#![allow(clippy::module_inception)]

use chrono::{
    DateTime, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone, Timelike, Utc,
};
use chrono_tz::Tz;

use super::plan_refusal::PlanRefusal;
use super::shift_instance::ShiftInstance;
use super::shift_template::{ShiftTemplate, ShiftTemplateInput};

const DAY_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftPlan {
    /// Sorted by `position`.
    templates: Vec<ShiftTemplate>,
    tz: Tz,
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).expect("a valid wall-clock time")
}

fn secs(t: NaiveTime) -> i64 {
    t.num_seconds_from_midnight() as i64
}

fn offset_secs(anchor: NaiveTime, t: NaiveTime) -> i64 {
    (secs(t) - secs(anchor)).rem_euclid(DAY_SECS)
}

fn duration_secs(start: NaiveTime, end: NaiveTime) -> i64 {
    let d = secs(end) - secs(start);
    if d <= 0 { d + DAY_SECS } else { d }
}

fn time_at(anchor: NaiveTime, offset: i64) -> NaiveTime {
    let total = (secs(anchor) + offset).rem_euclid(DAY_SECS) as u32;
    NaiveTime::from_num_seconds_from_midnight_opt(total, 0).expect("a valid wall-clock time")
}

/// Converts a branch-local wall-clock time to UTC; a boundary inside a DST gap starts at the end of the gap.
fn local_to_utc(tz: &Tz, naive: &NaiveDateTime) -> DateTime<Utc> {
    match tz.from_local_datetime(naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
        LocalResult::None => (1..=24 * 60)
            .find_map(|m| {
                tz.from_local_datetime(&(*naive + TimeDelta::minutes(m)))
                    .earliest()
            })
            .expect("DST gap shorter than 24 hours")
            .with_timezone(&Utc),
    }
}

impl ShiftPlan {
    pub const MAX_TEMPLATES: usize = 12;
    pub const MAX_NAME_CHARS: usize = 40;

    /// T5. Trims names and returns the inputs in the given order, or the first refusal found
    /// (row checks in row order, then 24-hour coverage from `S0`).
    pub fn validate(
        inputs: Vec<ShiftTemplateInput>,
    ) -> Result<Vec<ShiftTemplateInput>, PlanRefusal> {
        if inputs.is_empty() {
            return Err(PlanRefusal::None);
        }
        if inputs.len() > Self::MAX_TEMPLATES {
            return Err(PlanRefusal::TooMany);
        }

        let mut trimmed: Vec<ShiftTemplateInput> = Vec::with_capacity(inputs.len());
        for (i, mut row) in inputs.into_iter().enumerate() {
            row.name = row.name.trim().to_owned();
            if row.name.is_empty() {
                return Err(PlanRefusal::EmptyName(i));
            }
            if row.name.chars().count() > Self::MAX_NAME_CHARS {
                return Err(PlanRefusal::NameTooLong(i));
            }
            let lower = row.name.to_lowercase();
            if trimmed.iter().any(|prev| prev.name.to_lowercase() == lower) {
                return Err(PlanRefusal::DuplicateName(i));
            }
            if row.starts_at == row.ends_at {
                return Err(PlanRefusal::ZeroLength(i));
            }
            if let Some((bs, be)) = row.break_window {
                let shift_dur = duration_secs(row.starts_at, row.ends_at);
                let b_start = offset_secs(row.starts_at, bs);
                let b_end = b_start + duration_secs(bs, be);
                if b_start == 0 || b_end >= shift_dur {
                    return Err(PlanRefusal::BreakOutsideShift(i));
                }
            }
            trimmed.push(row);
        }

        let s0 = trimmed
            .iter()
            .map(|t| t.starts_at)
            .min()
            .expect("non-empty validated inputs");
        let mut segments = Vec::with_capacity(trimmed.len() * 2);
        for t in &trimmed {
            let start = offset_secs(s0, t.starts_at);
            let end = start + duration_secs(t.starts_at, t.ends_at);
            segments.push((start, end.min(DAY_SECS)));
            if end > DAY_SECS {
                segments.push((0, end - DAY_SECS));
            }
        }
        segments.sort_by_key(|&(start, _)| start);

        let mut covered_until = 0;
        for (start, end) in segments {
            if start > covered_until {
                return Err(PlanRefusal::Gap {
                    from: time_at(s0, covered_until),
                    to: time_at(s0, start),
                });
            }
            covered_until = covered_until.max(end);
        }
        if covered_until < DAY_SECS {
            return Err(PlanRefusal::Gap {
                from: time_at(s0, covered_until),
                to: s0,
            });
        }

        Ok(trimmed)
    }

    pub fn new(mut templates: Vec<ShiftTemplate>, tz: Tz) -> Self {
        templates.sort_by_key(|t| t.position);
        Self { templates, tz }
    }

    /// Owner 2026-11-04: *"โดยจะมี default 8ชม ต่อกะ"*. The seed and the settings screen's default length use it.
    pub const DEFAULT_SHIFT_HOURS: u32 = 8;

    /// The owner's defaults, 2026-11-04: three shifts of `DEFAULT_SHIFT_HOURS` from 06:00, 24-hour operation, no breaks,
    /// day cutover 06:00. The migration seeds the same rows (test S4).
    pub fn defaults() -> Vec<ShiftTemplateInput> {
        let h = Self::DEFAULT_SHIFT_HOURS;
        [("A", 6), ("B", 6 + h), ("C", 6 + 2 * h)]
            .into_iter()
            .map(|(name, start)| ShiftTemplateInput {
                id: None,
                name: name.to_owned(),
                starts_at: hm(start % 24, 0),
                ends_at: hm((start + h) % 24, 0),
                break_window: None,
            })
            .collect()
    }

    pub fn templates(&self) -> &[ShiftTemplate] {
        &self.templates
    }

    pub fn timezone(&self) -> Tz {
        self.tz
    }

    fn anchor(&self) -> Option<NaiveTime> {
        self.templates.iter().map(|t| t.starts_at).min()
    }

    fn at_offset(&self, date: NaiveDate, s0: NaiveTime, offset: i64) -> DateTime<Utc> {
        local_to_utc(&self.tz, &(date.and_time(s0) + TimeDelta::seconds(offset)))
    }

    /// T3: `template` on business `date`.
    pub fn instance(&self, template: &ShiftTemplate, date: NaiveDate) -> ShiftInstance {
        let s0 = self.anchor().unwrap_or(template.starts_at);
        let start_offset = offset_secs(s0, template.starts_at);
        let end_offset = start_offset + duration_secs(template.starts_at, template.ends_at);
        let break_window = template.break_window.map(|(bs, be)| {
            let b_start = start_offset + offset_secs(template.starts_at, bs);
            let b_end = b_start + duration_secs(bs, be);
            (
                self.at_offset(date, s0, b_start),
                self.at_offset(date, s0, b_end),
            )
        });
        ShiftInstance {
            template_id: template.id.clone(),
            name: template.name.clone(),
            business_date: date,
            starts_at: self.at_offset(date, s0, start_offset),
            ends_at: self.at_offset(date, s0, end_offset),
            break_window,
        }
    }

    /// T6: the instances whose `[starts_at, ends_at)` holds `now`, for business dates local today,
    /// yesterday and the day before; plan order, and for one template the earlier date first.
    pub fn covering(&self, now: DateTime<Utc>) -> Vec<ShiftInstance> {
        let today = now.with_timezone(&self.tz).date_naive();
        let yesterday = today.pred_opt().expect("valid predecessor date");
        let two_days_ago = yesterday.pred_opt().expect("valid predecessor date");
        let mut out = Vec::new();
        for t in &self.templates {
            for d in [two_days_ago, yesterday, today] {
                let inst = self.instance(t, d);
                if inst.starts_at <= now && now < inst.ends_at {
                    out.push(inst);
                }
            }
        }
        out
    }

    /// T7: the business date whose service span holds `now`; `None` outside every span.
    pub fn business_date_of(&self, now: DateTime<Utc>) -> Option<NaiveDate> {
        let today = now.with_timezone(&self.tz).date_naive();
        let yesterday = today.pred_opt()?;
        [yesterday, today].into_iter().find(|&d| {
            self.service_span(d)
                .is_some_and(|(start, end)| start <= now && now < end)
        })
    }

    /// T4: `[date + S0, date + S0 + 24 h)`; `None` for an empty plan.
    pub fn service_span(&self, date: NaiveDate) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let s0 = self.anchor()?;
        Some((
            self.at_offset(date, s0, 0),
            self.at_offset(date, s0, DAY_SECS),
        ))
    }

    /// `[required]` T9 (CS-20, design §2.10): the end of business day `date`, the end of `service_span(date)` (06:00 on
    /// `date + 1` with the defaults); `None` for an empty plan.
    pub fn business_day_end(&self, date: NaiveDate) -> Option<DateTime<Utc>> {
        self.service_span(date).map(|(_, end)| end)
    }

    /// The span of `business_date_of(now)`; `None` for an empty plan.
    pub fn span_for(&self, now: DateTime<Utc>) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        self.service_span(self.business_date_of(now)?)
    }

    /// True while `now` is inside the break of a covering instance of `template_id`.
    pub fn on_break(&self, template_id: &str, now: DateTime<Utc>) -> bool {
        self.covering(now).into_iter().any(|i| {
            i.template_id == template_id
                && i.break_window.is_some_and(|(bs, be)| bs <= now && now < be)
        })
    }
}
