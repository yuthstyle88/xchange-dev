//! `NotificationService` — writes notifications for producers (the `Notifications` port) and
//! answers the recipient's reads (design P8–P10, P15, §4.3).

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_kernel::notifications::{
    NotificationId, NotificationObject, NotificationRequest, Notifications,
};

use crate::application::template_registry::TemplateRegistry;
use crate::domain::channel::Channel;
use crate::domain::delivery::{DeliveryStatus, NO_CONSENT};
use crate::domain::error::NotifyError;
use crate::domain::notification::NotificationItem;
use crate::domain::template::{Locale, TemplateKind};

/// The `my_notifications` filter: `unread` and the `since` cursor (P15).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MyNotificationsQuery {
    pub unread_only: bool,
    pub since: Option<i64>,
}

/// The answer of `my_notifications`. `version` is the recipient's max `notifications.seq`
/// (0 with none); an unchanged `since` answers `Unchanged` (P15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MyNotifications {
    Unchanged {
        version: i64,
    },
    Changed {
        version: i64,
        items: Vec<NotificationItem>,
    },
}

#[derive(Clone)]
pub struct NotificationService {
    pool: PgPool,
    templates: Arc<TemplateRegistry>,
}

impl NotificationService {
    pub fn new(pool: PgPool, templates: Arc<TemplateRegistry>) -> Self {
        Self { pool, templates }
    }

    /// `notificationRead`: sets `read_at` only when it is NULL and answers the stored `read_at`.
    /// Another user's notification → `OutOfScope`.
    pub async fn mark_read(
        &self,
        user_id: &str,
        id: &NotificationId,
    ) -> Result<DateTime<Utc>, NotifyError> {
        let user_uuid = Uuid::parse_str(user_id)
            .map_err(|_| NotifyError::Store(AppError::Validation("invalid user_id".into())))?;
        let notif_uuid = Uuid::parse_str(&id.0).map_err(|_| NotifyError::NotFound)?;

        let row: Option<(Uuid, Option<DateTime<Utc>>)> =
            sqlx::query_as("SELECT recipient_user_id, read_at FROM notifications WHERE id = $1")
                .bind(notif_uuid)
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;

        let (recipient, read_at) = row.ok_or(NotifyError::NotFound)?;
        if recipient != user_uuid {
            return Err(NotifyError::OutOfScope);
        }

        if let Some(existing) = read_at {
            return Ok(existing);
        }

        let updated: DateTime<Utc> = sqlx::query_scalar(
            "UPDATE notifications SET read_at = now() \
             WHERE id = $1 AND read_at IS NULL \
             RETURNING read_at",
        )
        .bind(notif_uuid)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(updated)
    }

    /// `notificationReadAll`: marks the user's own unread notifications; answers how many.
    pub async fn mark_all_read(&self, user_id: &str) -> Result<u64, NotifyError> {
        let user_uuid = Uuid::parse_str(user_id)
            .map_err(|_| NotifyError::Store(AppError::Validation("invalid user_id".into())))?;

        let res = sqlx::query(
            "UPDATE notifications SET read_at = now() \
             WHERE recipient_user_id = $1 AND read_at IS NULL",
        )
        .bind(user_uuid)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(res.rows_affected())
    }

    /// The view `my_notifications`: the user's own notifications, newest first.
    pub async fn my_notifications(
        &self,
        user_id: &str,
        query: &MyNotificationsQuery,
    ) -> Result<MyNotifications, NotifyError> {
        let user_uuid = Uuid::parse_str(user_id)
            .map_err(|_| NotifyError::Store(AppError::Validation("invalid user_id".into())))?;

        let version: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) FROM notifications WHERE recipient_user_id = $1",
        )
        .bind(user_uuid)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;

        if query.since == Some(version) {
            return Ok(MyNotifications::Unchanged { version });
        }

        let rows = sqlx::query(
            "SELECT id::text, seq, locale, template_key, object_type, object_id, params, read_at, created_at \
             FROM notifications \
             WHERE recipient_user_id = $1 \
               AND ($2 = false OR read_at IS NULL) \
             ORDER BY seq DESC",
        )
        .bind(user_uuid)
        .bind(query.unread_only)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get(0);
            let seq: i64 = row.get(1);
            let locale_str: String = row.get(2);
            let template_key: String = row.get(3);
            let object_type: Option<String> = row.get(4);
            let object_id: Option<String> = row.get(5);
            let params_val: serde_json::Value = row.get(6);
            let read_at: Option<DateTime<Utc>> = row.get(7);
            let created_at: DateTime<Utc> = row.get(8);

            let locale = match locale_str.as_str() {
                "zh" => Locale::Zh,
                "en" => Locale::En,
                _ => Locale::Th,
            };

            let params: BTreeMap<String, String> =
                serde_json::from_value(params_val).unwrap_or_default();

            let (title, body) = match self.templates.get(&template_key) {
                Some(t) => {
                    let r = t.render(locale, &params);
                    (r.title, r.body)
                }
                None => (String::new(), String::new()),
            };

            let object = match (object_type, object_id) {
                (Some(ot), Some(oi)) => Some(NotificationObject {
                    object_type: ot,
                    object_id: oi,
                }),
                _ => None,
            };

            items.push(NotificationItem {
                id: NotificationId(id),
                seq,
                locale,
                title,
                body,
                object,
                read: read_at.is_some(),
                created_at,
            });
        }

        Ok(MyNotifications::Changed { version, items })
    }
}

impl Notifications for NotificationService {
    fn notify_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        request: &'a NotificationRequest,
    ) -> BoxFuture<'a, AppResult<NotificationId>> {
        Box::pin(async move {
            let template = self.templates.get(request.template_key()).ok_or_else(|| {
                AppError::Validation(format!("unknown template: {}", request.template_key()))
            })?;

            let recipient_uuid = Uuid::parse_str(request.recipient_user_id())
                .map_err(|e| AppError::Validation(format!("invalid recipient_user_id: {e}")))?;

            let existing: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM notifications \
                 WHERE recipient_user_id = $1 AND event_id = $2 AND template_key = $3",
            )
            .bind(recipient_uuid)
            .bind(request.event_id())
            .bind(request.template_key())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            if let Some(id) = existing {
                return Ok(NotificationId(id.to_string()));
            }

            let org_id: Option<Uuid> =
                sqlx::query_scalar("SELECT organization_id FROM users WHERE id = $1")
                    .bind(recipient_uuid)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

            let org_id =
                org_id.ok_or_else(|| AppError::Validation("recipient user not found".into()))?;

            let customer_row: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT id, preferred_language FROM customers \
                 WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(recipient_uuid)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let (customer_id, preferred_language) = match customer_row {
                Some((cid, lang)) => (Some(cid), lang),
                None => (None, None),
            };

            let locale_str = match preferred_language.as_deref() {
                Some("zh") => "zh",
                Some("en") => "en",
                _ => "th",
            };

            let (delivery_status, skipped_reason) = match template.kind() {
                TemplateKind::Marketing => {
                    let has_consent = if let Some(ref cid) = customer_id {
                        let granted_rows: Vec<bool> = sqlx::query_scalar(
                            "SELECT DISTINCT ON (channel) granted \
                             FROM consents \
                             WHERE customer_id = $1 AND purpose = 'MARKETING' \
                             ORDER BY channel, effective_at DESC",
                        )
                        .bind(cid)
                        .fetch_all(&mut **tx)
                        .await
                        .map_err(AppError::Database)?;

                        !granted_rows.is_empty() && granted_rows.iter().any(|&g| g)
                    } else {
                        false
                    };

                    if has_consent {
                        (DeliveryStatus::Sent, None)
                    } else {
                        (DeliveryStatus::Skipped, Some(NO_CONSENT.to_string()))
                    }
                }
                TemplateKind::Transactional => (DeliveryStatus::Sent, None),
            };

            let notif_id = Uuid::new_v4();
            let object_type = request.object().map(|o| o.object_type.as_str());
            let object_id = request.object().map(|o| o.object_id.as_str());
            let params_json = serde_json::to_value(request.params())
                .map_err(|e| AppError::Internal(format!("params serialization failed: {e}")))?;

            let inserted_id: Option<Uuid> = sqlx::query_scalar(
                "INSERT INTO notifications ( \
                     id, organization_id, recipient_user_id, event_id, template_key, \
                     template_version, locale, object_type, object_id, params \
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
                 ON CONFLICT (recipient_user_id, event_id, template_key) DO NOTHING \
                 RETURNING id",
            )
            .bind(notif_id)
            .bind(org_id)
            .bind(recipient_uuid)
            .bind(request.event_id())
            .bind(request.template_key())
            .bind(template.version())
            .bind(locale_str)
            .bind(object_type)
            .bind(object_id)
            .bind(params_json)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let final_id = match inserted_id {
                Some(id) => {
                    let delivery_id = Uuid::new_v4();
                    sqlx::query(
                        "INSERT INTO notification_deliveries ( \
                             id, organization_id, notification_id, channel, status, \
                             attempt_no, skipped_reason \
                         ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    )
                    .bind(delivery_id)
                    .bind(org_id)
                    .bind(id)
                    .bind(Channel::InApp.as_str())
                    .bind(delivery_status.as_str())
                    .bind(0)
                    .bind(skipped_reason)
                    .execute(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    id
                }
                None => {
                    let existing_id: Uuid = sqlx::query_scalar(
                        "SELECT id FROM notifications \
                         WHERE recipient_user_id = $1 AND event_id = $2 AND template_key = $3",
                    )
                    .bind(recipient_uuid)
                    .bind(request.event_id())
                    .bind(request.template_key())
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    existing_id
                }
            };

            Ok(NotificationId(final_id.to_string()))
        })
    }
}
