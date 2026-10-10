//! `Notification` — one row of `notifications`, rendered for its recipient.

use chrono::{DateTime, Utc};
use xc_kernel::notifications::{NotificationId, NotificationObject};

use super::template::Locale;

/// One notification as the recipient's `my_notifications` shows it: title and body rendered in
/// the locale stored on the row, never the reader's page locale (design P10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationItem {
    pub id: NotificationId,
    pub seq: i64,
    pub locale: Locale,
    pub title: String,
    pub body: String,
    pub object: Option<NotificationObject>,
    pub read: bool,
    pub created_at: DateTime<Utc>,
}
