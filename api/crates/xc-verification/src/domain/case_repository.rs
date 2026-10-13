//! `CaseRepository` (design §5): what the case side reads and writes, inside the caller's transaction.
//!
//! The methods are the implementer's (VF-3 owns this file); the provided tests reach the store only through
//! `PgCaseRepository::new` and `CaseService`.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::case::case_error::CaseError;
use crate::domain::case::verification_case::VerificationCase;
use crate::domain::document::verification_document::VerificationDocument;
use crate::domain::policy::verification_policy::VerificationPolicy;

pub trait CaseRepository: Send + Sync {
    /// Look up customer's organisation and user ID.
    #[allow(clippy::type_complexity)]
    fn find_customer<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<(Uuid, Option<Uuid>)>, CaseError>>;

    /// Find latest published policy for an organisation.
    fn find_latest_policy<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> BoxFuture<'a, Result<Option<VerificationPolicy>, CaseError>>;

    /// Find specific policy version for an organisation.
    fn find_policy<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        version: i32,
    ) -> BoxFuture<'a, Result<Option<VerificationPolicy>, CaseError>>;

    /// Find an open case for a customer (DRAFT, SUBMITTED, UNDER_REVIEW, NEEDS_UPDATE).
    fn find_open_case_by_customer<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<VerificationCase>, CaseError>>;

    /// Insert a new case. Returns Some(case) if inserted, or None if ON CONFLICT DO NOTHING triggered.
    fn insert_case<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case: &'a VerificationCase,
    ) -> BoxFuture<'a, Result<Option<VerificationCase>, CaseError>>;

    /// Find a case by id, locking the row FOR UPDATE.
    fn find_case_by_id_for_update<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
    ) -> BoxFuture<'a, Result<Option<VerificationCase>, CaseError>>;

    /// Update case to SUBMITTED status.
    fn update_case_submitted<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
        submitted_at: DateTime<Utc>,
        version: i32,
    ) -> BoxFuture<'a, Result<(), CaseError>>;

    /// Find a document by its encrypted object ref.
    fn find_document_by_ref<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_ref: &'a str,
    ) -> BoxFuture<'a, Result<Option<VerificationDocument>, CaseError>>;

    /// Find a document by its ID.
    fn find_document_by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<VerificationDocument>, CaseError>>;

    /// List active (neither removed nor purged) documents for a case.
    fn list_active_documents_by_case<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
    ) -> BoxFuture<'a, Result<Vec<VerificationDocument>, CaseError>>;

    /// Insert document metadata.
    fn insert_document<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        doc: &'a VerificationDocument,
    ) -> BoxFuture<'a, Result<(), CaseError>>;

    /// Mark a document as removed (sets removed_at, clears encrypted_object_ref).
    fn remove_document<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), CaseError>>;
}
