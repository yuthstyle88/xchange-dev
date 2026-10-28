//! Staff corrections §4 A (`docs/design/staff-corrections.md`): a manager closes another employee's open
//! clock-in from an earlier day, with the real clock-out time and a reason (`attendanceCloseFor`).

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use pos_error::AppError;
use xc_auth::domain::access::Refusal;

use crate::domain::custody::entities::attendance::Attendance;

/// The longest reason, in UTF-16 units (SC-D10).
pub const REASON_MAX_UTF16: usize = 500;

/// What the manager asks: close `attendance_id` at `clock_out`, because of `reason`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseFor {
    pub attendance_id: String,
    pub clock_out: DateTime<Utc>,
    pub reason: String,
}

/// Why `CloseFor::check` refuses (A3–A10, §5), in the order it checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseForRefusal {
    /// A3: the row is the caller's own.
    SelfClose,
    /// A4: the row already has a clock-out.
    NotOpen,
    /// A5: the reason is blank after trim.
    ReasonRequired,
    /// A6: the reason is longer than [`REASON_MAX_UTF16`].
    ReasonTooLong,
    /// A8: the clock-out is before the clock-in.
    BeforeClockIn,
    /// A9: the clock-out is after `now`.
    InFuture,
    /// A10: the clock-out's date in the branch timezone is today or later there.
    NotEarlierDay,
}

impl CloseForRefusal {
    /// The v3 refusal code (§5).
    pub fn code(self) -> &'static str {
        match self {
            CloseForRefusal::SelfClose => "selfClose",
            CloseForRefusal::NotOpen => "notOpen",
            CloseForRefusal::ReasonRequired => "reasonRequired",
            CloseForRefusal::ReasonTooLong => "validationFailed",
            CloseForRefusal::BeforeClockIn => "beforeClockIn",
            CloseForRefusal::InFuture => "inFuture",
            CloseForRefusal::NotEarlierDay => "notEarlierDay",
        }
    }

    /// The input field the refusal is about, if any (§5).
    pub fn field(self) -> Option<&'static str> {
        match self {
            CloseForRefusal::SelfClose | CloseForRefusal::NotOpen => None,
            CloseForRefusal::ReasonRequired | CloseForRefusal::ReasonTooLong => Some("reason"),
            CloseForRefusal::BeforeClockIn
            | CloseForRefusal::InFuture
            | CloseForRefusal::NotEarlierDay => Some("clockOut"),
        }
    }
}

/// The answer of `BranchDayService::close_for` when it does not close the row.
#[derive(Debug)]
pub enum CloseForError {
    /// A1: no `AttendanceCloseFor` on the branch (`roleNotAllowed`, `outOfScope`, `signIn`).
    NotAllowed(Refusal),
    /// A2: no attendance row with that id at this branch.
    NotFound,
    /// A3–A10.
    Refused(CloseForRefusal),
    /// Anything else (database, …), unchanged.
    Other(AppError),
}

impl From<AppError> for CloseForError {
    fn from(e: AppError) -> Self {
        CloseForError::Other(e)
    }
}

impl CloseFor {
    /// A3–A10 on `row`, first fault wins: self, open, reason, before clock-in, future, earlier day.
    /// `caller_id` is the manager's user id; `today` is decided from `now` in `tz`.
    pub fn check(
        &self,
        row: &Attendance,
        caller_id: &str,
        now: DateTime<Utc>,
        tz: Tz,
    ) -> Result<(), CloseForRefusal> {
        todo!(
            "SC-A1: check {} for {caller_id} on {} at {now} in {tz}",
            self.attendance_id,
            row.id
        )
    }
}
