//! `AuditLog`: append-only redacted audit trail port and types.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use serde_json::Value;
use sqlx::{Postgres, Transaction};

/// An audit event to be recorded.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub actor_id: String,
    pub actor_role: String,
    pub scope: String,
    pub action: String,
    pub object_ref: String,
    pub branch_id: Option<String>,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// An audit entry retrieved from the log.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub id: i64,
    pub at: DateTime<Utc>,
    pub event: AuditEvent,
}

/// Append-only audit log port.
pub trait AuditLog: Send + Sync {
    /// Appends an audit event to the log in the caller's transaction, returning the new id.
    ///
    /// The log redacts `before` and `after` payloads before writing them.
    fn append<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        event: &'a AuditEvent,
    ) -> BoxFuture<'a, AppResult<i64>>;

    /// Reads a page of audit entries strictly ascending by id (keyset pagination).
    fn page<'a>(
        &'a self,
        after_id: Option<i64>,
        limit: i64,
    ) -> BoxFuture<'a, AppResult<Vec<AuditEntry>>>;
}
