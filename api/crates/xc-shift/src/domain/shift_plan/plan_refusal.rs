//! Why a shift list cannot be saved (configurable-shifts.md §2.2 T5, §2.9).
//!
//! `reason()` is the wire `reason` of `422 shiftPlanInvalid`; `detail()` is the offending row's index, or the uncovered
//! window `HH:MM-HH:MM` for `Gap`.

use chrono::NaiveTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanRefusal {
    #[error("ต้องมีอย่างน้อยหนึ่งกะ")]
    None,
    #[error("กำหนดกะได้ไม่เกิน 12 กะ")]
    TooMany,
    #[error("กรุณาตั้งชื่อกะ")]
    EmptyName(usize),
    #[error("ชื่อกะยาวเกิน 40 ตัวอักษร")]
    NameTooLong(usize),
    #[error("ชื่อกะซ้ำกัน")]
    DuplicateName(usize),
    #[error("เวลาเริ่มและเวลาเลิกกะต้องไม่เท่ากัน")]
    ZeroLength(usize),
    #[error("กะทั้งหมดต้องอยู่ภายใน 24 ชั่วโมงนับจากกะแรก")]
    SpanOver24h,
    /// Owner 2026-11-04: the plan must cover all 24 hours. The first uncovered window from the cutover.
    #[error("ช่วง {}–{} ไม่มีกะ", .from.format("%H:%M"), .to.format("%H:%M"))]
    Gap { from: NaiveTime, to: NaiveTime },
    #[error("กรุณาระบุเวลาพักให้ครบทั้งเริ่มและสิ้นสุด")]
    BreakIncomplete(usize),
    #[error("เวลาพักต้องอยู่ภายในเวลากะ")]
    BreakOutsideShift(usize),
}

impl PlanRefusal {
    pub fn reason(&self) -> &'static str {
        match self {
            PlanRefusal::None => "none",
            PlanRefusal::TooMany => "tooMany",
            PlanRefusal::EmptyName(_) => "emptyName",
            PlanRefusal::NameTooLong(_) => "nameTooLong",
            PlanRefusal::DuplicateName(_) => "duplicateName",
            PlanRefusal::ZeroLength(_) => "zeroLength",
            PlanRefusal::SpanOver24h => "spanOver24h",
            PlanRefusal::BreakIncomplete(_) => "breakIncomplete",
            PlanRefusal::BreakOutsideShift(_) => "breakOutsideShift",
            PlanRefusal::Gap { .. } => "gap",
        }
    }

    pub fn index(&self) -> Option<usize> {
        match *self {
            PlanRefusal::EmptyName(i)
            | PlanRefusal::NameTooLong(i)
            | PlanRefusal::DuplicateName(i)
            | PlanRefusal::ZeroLength(i)
            | PlanRefusal::BreakIncomplete(i)
            | PlanRefusal::BreakOutsideShift(i) => Some(i),
            PlanRefusal::None
            | PlanRefusal::TooMany
            | PlanRefusal::SpanOver24h
            | PlanRefusal::Gap { .. } => None,
        }
    }

    /// The wire detail: the row index, the uncovered window `HH:MM-HH:MM` for `Gap`, else empty.
    pub fn detail(&self) -> String {
        match *self {
            PlanRefusal::Gap { from, to } => {
                format!("{}-{}", from.format("%H:%M"), to.format("%H:%M"))
            }
            _ => self.index().map(|i| i.to_string()).unwrap_or_default(),
        }
    }
}
