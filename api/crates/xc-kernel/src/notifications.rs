//! `Notifications` — the port producers write a customer notification through (design
//! `customer-app.md` P8, §5).
//!
//! Queue (APP-10), bookings (#555) and rate alerts (#561) call `notify_in` inside their own
//! transaction, so none of them depends on `xc-notify`. `xc_notify::application::notification_service::
//! NotificationService` implements it (APP-3); the root package wires it.

use std::collections::BTreeMap;

use futures::future::BoxFuture;
use pos_error::AppResult;

/// `notifications.id`, a uuid in text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NotificationId(pub String);

/// Why a [`NotificationRequest`] could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotificationRequestError {
    /// A required text field is empty.
    #[error("{0} is empty")]
    Empty(&'static str),
    /// A param value holds personal data (`redact_text` would change it). Design P10: params hold
    /// references and display values, never PII.
    #[error("param {0} holds personal data")]
    PersonalData(String),
}

/// The link target of a notification (C13 *"ลิงก์ถึง object"*): `notifications.object_type/object_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationObject {
    pub object_type: String,
    pub object_id: String,
}

/// What a producer asks to be notified. Built only through [`NotificationRequest::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRequest {
    recipient_user_id: String,
    event_id: String,
    template_key: String,
    object: Option<NotificationObject>,
    params: BTreeMap<String, String>,
}

impl NotificationRequest {
    /// Refuses an empty `recipient_user_id`, `event_id` or `template_key`, and any param value that
    /// `crate::pii::redact_text` would change.
    pub fn new(
        recipient_user_id: impl Into<String>,
        event_id: impl Into<String>,
        template_key: impl Into<String>,
        object: Option<NotificationObject>,
        params: BTreeMap<String, String>,
    ) -> Result<Self, NotificationRequestError> {
        let recipient_user_id = recipient_user_id.into();
        if recipient_user_id.is_empty() {
            return Err(NotificationRequestError::Empty("recipient_user_id"));
        }
        let event_id = event_id.into();
        if event_id.is_empty() {
            return Err(NotificationRequestError::Empty("event_id"));
        }
        let template_key = template_key.into();
        if template_key.is_empty() {
            return Err(NotificationRequestError::Empty("template_key"));
        }

        for (k, v) in &params {
            let redacted = crate::pii::redact_text(v);
            if redacted != *v {
                return Err(NotificationRequestError::PersonalData(k.clone()));
            }
        }

        Ok(Self {
            recipient_user_id,
            event_id,
            template_key,
            object,
            params,
        })
    }

    /// Builds a request checking that any `Text` param contains no PII, while `Display` params skip the check.
    pub fn with_params(
        recipient_user_id: impl Into<String>,
        event_id: impl Into<String>,
        template_key: impl Into<String>,
        object: Option<NotificationObject>,
        params: crate::notification_params::NotificationParams,
    ) -> Result<Self, NotificationRequestError> {
        let recipient_user_id = recipient_user_id.into();
        if recipient_user_id.is_empty() {
            return Err(NotificationRequestError::Empty("recipient_user_id"));
        }
        let event_id = event_id.into();
        if event_id.is_empty() {
            return Err(NotificationRequestError::Empty("event_id"));
        }
        let template_key = template_key.into();
        if template_key.is_empty() {
            return Err(NotificationRequestError::Empty("template_key"));
        }

        let mut stored_params = BTreeMap::new();
        for (k, v) in params.values {
            match v {
                crate::notification_params::ParamValue::Text(t) => {
                    let redacted = crate::pii::redact_text(&t);
                    if redacted != t {
                        return Err(NotificationRequestError::PersonalData(k));
                    }
                    stored_params.insert(k, t);
                }
                crate::notification_params::ParamValue::Display(d) => {
                    stored_params.insert(k, d.as_str().to_string());
                }
            }
        }

        Ok(Self {
            recipient_user_id,
            event_id,
            template_key,
            object,
            params: stored_params,
        })
    }

    pub fn recipient_user_id(&self) -> &str {
        &self.recipient_user_id
    }

    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    pub fn template_key(&self) -> &str {
        &self.template_key
    }

    pub fn object(&self) -> Option<&NotificationObject> {
        self.object.as_ref()
    }

    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }
}

/// The producers' port. One notification per (recipient, event, template): a repeat returns the
/// existing id and writes nothing (design §4.3).
pub trait Notifications: Send + Sync {
    /// Writes the notification and its IN_APP delivery inside `tx`; a rollback of `tx` writes neither.
    fn notify_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        request: &'a NotificationRequest,
    ) -> BoxFuture<'a, AppResult<NotificationId>>;
}
