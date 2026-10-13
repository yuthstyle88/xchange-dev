//! `DocumentAccess` (design V6, V8, V10, §5): who may get a link to a `VerificationDocument`, and serving one link.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::permission::Permission;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};
use xc_kernel::ids::BranchId;

use super::download_link::{DownloadLink, LinkError};

/// Where a link's content is served (design §4.5 `GET /api/verification/content/{token}`).
pub const CONTENT_PATH: &str = "/api/verification/content/";

/// `{url, expiresAt}` (design V10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedLink {
    /// `CONTENT_PATH` followed by the token.
    pub url: String,
    pub expires_at: DateTime<Utc>,
}

/// One link's content: the bytes the store returns (decrypted by it) and their type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentContent {
    pub bytes: Vec<u8>,
    pub mime: String,
}

impl DocumentContent {
    /// The response headers the content route sends (V10, as `files.rs` does for the staff door):
    /// `Content-Type`, `Cache-Control: private, no-store`, `X-Content-Type-Options: nosniff`,
    /// `Content-Disposition: attachment`.
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Content-Type", self.mime.clone()),
            ("Cache-Control", "private, no-store".to_string()),
            ("X-Content-Type-Options", "nosniff".to_string()),
            ("Content-Disposition", "attachment".to_string()),
        ]
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AccessError {
    /// Not the case's reviewer in scope, nor its own customer (`outOfScope`).
    #[error("out of scope")]
    OutOfScope,
    /// No such document, or it was removed or purged.
    #[error("no such document")]
    NotFound,
    /// The link was used after its lifetime (`linkExpired`).
    #[error("the link has expired")]
    LinkExpired,
    /// Another session's link, or not a link this server signed.
    #[error("the link is refused")]
    LinkRefused,
    #[error("internal: {0}")]
    Internal(String),
}

impl AccessError {
    /// The wire code (design §10 errors; #1393 §4).
    pub fn code(&self) -> &'static str {
        match self {
            Self::OutOfScope => "outOfScope",
            Self::NotFound => "notFound",
            Self::LinkExpired => "linkExpired",
            Self::LinkRefused => "linkRefused",
            Self::Internal(_) => "serverError",
        }
    }
}

type DocMetaTuple = (
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Uuid,
    Option<String>,
);

type DocOwnerTuple = (
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Uuid,
    String,
    Option<String>,
);

type DocContentTuple = (
    Option<String>,
    String,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

pub struct DocumentAccess {
    pool: PgPool,
    store: Arc<dyn DocumentStore>,
    audit: Arc<dyn AuditLog>,
    link: DownloadLink,
}

impl DocumentAccess {
    pub fn new(
        pool: PgPool,
        store: Arc<dyn DocumentStore>,
        audit: Arc<dyn AuditLog>,
        link: DownloadLink,
    ) -> Self {
        Self {
            pool,
            store,
            audit,
            link,
        }
    }

    /// F02: a link for `actor` when they hold `VerificationReview` in the scope of the document's case (an
    /// organisation-wide row, or the case's own branch for an at-branch case, V17); `OutOfScope` otherwise.
    pub async fn link_for_reviewer(
        &self,
        actor: &Actor,
        session: &str,
        document_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<IssuedLink, AccessError> {
        let query_str = "SELECT d.encrypted_object_ref, d.removed_at, d.purged_at, c.organization_id AS case_org_id, c.branch_id \
                         FROM verification_documents d \
                         JOIN verification_cases c ON c.id = d.case_id \
                         WHERE d.id = $1";
        let row: Option<DocMetaTuple> = sqlx::query_as(query_str)
            .bind(document_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AccessError::Internal(e.to_string()))?;

        let (encrypted_object_ref, removed_at, purged_at, case_org_id, branch_id) =
            row.ok_or(AccessError::NotFound)?;

        if encrypted_object_ref.is_none() || removed_at.is_some() || purged_at.is_some() {
            return Err(AccessError::NotFound);
        }

        if actor.organization_id != case_org_id.to_string() {
            return Err(AccessError::OutOfScope);
        }

        let scope = BranchScope::of(actor, Permission::VerificationReview, now);
        let in_scope = match &branch_id {
            None => matches!(scope, BranchScope::All),
            Some(b) => scope.contains(&BranchId::new(b)),
        };
        if !in_scope {
            return Err(AccessError::OutOfScope);
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AccessError::Internal(e.to_string()))?;

        let actor_role = actor
            .roles
            .first()
            .map(|r| r.role.as_str().to_string())
            .unwrap_or_else(|| "staff_role".to_string());

        let event = AuditEvent {
            actor_id: actor.user_id.clone(),
            actor_role,
            scope: case_org_id.to_string(),
            action: "verification_document.link".to_string(),
            object_ref: document_id.to_string(),
            branch_id,
            before: None,
            after: None,
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(|e| AccessError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| AccessError::Internal(e.to_string()))?;

        let signed = self.link.sign(document_id, &actor.user_id, session, now);
        Ok(IssuedLink {
            url: format!("{CONTENT_PATH}{}", signed.token),
            expires_at: signed.expires_at,
        })
    }

    /// C10: a link for `actor` when `actor.customer_id` is the case's customer; `OutOfScope` otherwise.
    pub async fn link_for_owner(
        &self,
        actor: &Actor,
        session: &str,
        document_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<IssuedLink, AccessError> {
        let query_str = "SELECT d.encrypted_object_ref, d.removed_at, d.purged_at, c.organization_id AS case_org_id, c.customer_id, c.branch_id \
                         FROM verification_documents d \
                         JOIN verification_cases c ON c.id = d.case_id \
                         WHERE d.id = $1";
        let row: Option<DocOwnerTuple> = sqlx::query_as(query_str)
            .bind(document_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AccessError::Internal(e.to_string()))?;

        let (encrypted_object_ref, removed_at, purged_at, case_org_id, customer_id, branch_id) =
            row.ok_or(AccessError::NotFound)?;

        if encrypted_object_ref.is_none() || removed_at.is_some() || purged_at.is_some() {
            return Err(AccessError::NotFound);
        }

        if actor.organization_id != case_org_id.to_string() {
            return Err(AccessError::OutOfScope);
        }

        match &actor.customer_id {
            Some(cid) if cid == &customer_id => {}
            _ => return Err(AccessError::OutOfScope),
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AccessError::Internal(e.to_string()))?;

        let actor_role = actor
            .roles
            .first()
            .map(|r| r.role.as_str().to_string())
            .unwrap_or_else(|| "owner_role".to_string());

        let event = AuditEvent {
            actor_id: actor.user_id.clone(),
            actor_role,
            scope: case_org_id.to_string(),
            action: "verification_document.link".to_string(),
            object_ref: document_id.to_string(),
            branch_id,
            before: None,
            after: None,
        };
        self.audit
            .append(&mut tx, &event)
            .await
            .map_err(|e| AccessError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| AccessError::Internal(e.to_string()))?;

        let signed = self.link.sign(document_id, &actor.user_id, session, now);
        Ok(IssuedLink {
            url: format!("{CONTENT_PATH}{}", signed.token),
            expires_at: signed.expires_at,
        })
    }

    /// The content of the link `token` for `session` at `now`: `LinkExpired` after its lifetime, `LinkRefused` for
    /// another session or a token this server did not sign, `NotFound` when the document was removed or purged.
    pub async fn open(
        &self,
        token: &str,
        session: &str,
        now: DateTime<Utc>,
    ) -> Result<DocumentContent, AccessError> {
        let claims = self.link.verify(token, session, now).map_err(|e| match e {
            LinkError::Expired => AccessError::LinkExpired,
            LinkError::OtherSession => AccessError::LinkRefused,
            LinkError::Invalid => AccessError::LinkRefused,
        })?;

        let query_str = "SELECT encrypted_object_ref, mime, removed_at, purged_at \
                         FROM verification_documents \
                         WHERE id = $1";
        let row: Option<DocContentTuple> = sqlx::query_as(query_str)
            .bind(claims.document_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AccessError::Internal(e.to_string()))?;

        let (encrypted_object_ref, mime, removed_at, purged_at) =
            row.ok_or(AccessError::NotFound)?;

        if removed_at.is_some() || purged_at.is_some() {
            return Err(AccessError::NotFound);
        }

        let doc_ref = encrypted_object_ref.ok_or(AccessError::NotFound)?;

        let bytes = self.store.get(&doc_ref).await.map_err(|e| match e {
            DocumentStoreError::NotFound(_) => AccessError::NotFound,
            other => AccessError::Internal(other.to_string()),
        })?;

        Ok(DocumentContent { bytes, mime })
    }
}
