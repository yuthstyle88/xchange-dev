//! Service window value object representing branch opening hours for custody.

use super::custody_time::CustodyTime;

/// The service window in which custody operations may proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ServiceWindow {
    #[default]
    NotSet,
    Closed,
    Open {
        open: CustodyTime,
        close: CustodyTime,
    },
}

impl ServiceWindow {
    /// Beginning a transaction: within `[open, close - 10 min)`.
    pub fn allows_begin(&self, now: CustodyTime) -> bool {
        match *self {
            ServiceWindow::Open { open, close } => {
                let cutoff = close.epoch_millis() - 10 * 60 * 1000;
                open.epoch_millis() <= now.epoch_millis() && now.epoch_millis() < cutoff
            }
            ServiceWindow::NotSet | ServiceWindow::Closed => false,
        }
    }

    /// Starting the service: within `[open, close - 10 min)`.
    pub fn allows_start(&self, now: CustodyTime) -> bool {
        self.allows_begin(now)
    }

    /// A save: within `[open, close)`.
    pub fn allows_save(&self, now: CustodyTime) -> bool {
        match *self {
            ServiceWindow::Open { open, close } => {
                open.epoch_millis() <= now.epoch_millis()
                    && now.epoch_millis() < close.epoch_millis()
            }
            ServiceWindow::NotSet | ServiceWindow::Closed => false,
        }
    }

    /// From `close - 10 min` onward (or if not open), only transactions begun before may save.
    pub fn needs_draft(&self, now: CustodyTime) -> bool {
        match *self {
            ServiceWindow::Open { close, .. } => {
                let cutoff = close.epoch_millis() - 10 * 60 * 1000;
                now.epoch_millis() >= cutoff
            }
            ServiceWindow::NotSet | ServiceWindow::Closed => true,
        }
    }
}
