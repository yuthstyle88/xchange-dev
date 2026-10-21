//! `KioskDayClose` — the kiosk's step at branch day close (#4147, design `queue-day-close.md` §7).
//!
//! `QueueDayHook` calls it first, inside the day close's transaction, so `xc-queue` never names
//! `xc-kiosk`. `xc_kiosk::application::kiosk::day_close::KioskDayCloser` implements it.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};

use crate::ids::BranchId;

pub trait KioskDayClose: Send + Sync {
    /// Cancels every unfinished request of `branch` (`WAITING`, `ACCEPTED`, `ID_CHECKED`) with
    /// `DAY_CLOSED` inside `tx`, ending each one's ticket (SERVING included) as
    /// `kioskCancelled:DAY_CLOSED`. Answers the ids it cancelled; a request the kiosk machine
    /// refuses is skipped. An error rolls the day close back.
    fn close_day_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<String>>>;
}
