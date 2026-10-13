//! `PgCaseRepository` (design §5): [`CaseRepository`] on the three verification tables. Bytes never reach the
//! database: the only `bytea` is the checksum (ticket §4 item 2).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::identity_type::IdentityType;

use crate::domain::case::case_error::CaseError;
use crate::domain::case::case_status::CaseStatus;
use crate::domain::case::decision_reason::DecisionReason;
use crate::domain::case::provider::Provider;
use crate::domain::case::verification_case::VerificationCase;
use crate::domain::case_repository::CaseRepository;
use crate::domain::document::access_class::AccessClass;
use crate::domain::document::verification_document::VerificationDocument;
use crate::domain::policy::verification_policy::VerificationPolicy;

pub struct PgCaseRepository {
    pool: PgPool,
}

impl PgCaseRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The pool this repository writes through.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

type CaseRow = (
    Uuid,
    Uuid,
    String,
    String,
    String,
    i32,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<Uuid>,
    Option<String>,
    i32,
);

fn map_case(row: CaseRow) -> VerificationCase {
    let (
        id,
        organization_id,
        customer_id,
        provider_str,
        status_str,
        policy_version,
        branch_id_str,
        submitted_at,
        decided_at,
        expires_at,
        reviewed_by,
        reason_str,
        version,
    ) = row;
    VerificationCase {
        id,
        organization_id,
        customer_id,
        provider: Provider::parse(&provider_str).unwrap_or(Provider::Staff),
        status: CaseStatus::parse(&status_str).expect("a known status"),
        policy_version,
        branch_id: branch_id_str.map(BranchId::new),
        submitted_at,
        decided_at,
        expires_at,
        reviewed_by,
        decision_reason: reason_str.and_then(|r| DecisionReason::parse(&r)),
        version,
    }
}

type DocRow = (
    Uuid,
    Uuid,
    String,
    Option<String>,
    Option<String>,
    Vec<u8>,
    String,
    i32,
    DateTime<Utc>,
    Uuid,
    Option<DateTime<Utc>>,
    String,
    Option<DateTime<Utc>>,
);

fn map_doc(row: DocRow) -> VerificationDocument {
    let (
        id,
        case_id,
        doc_type_str,
        encrypted_object_ref,
        key_id,
        checksum,
        mime,
        size_bytes,
        uploaded_at,
        uploaded_by,
        retention_until,
        access_class_str,
        removed_at,
    ) = row;
    let document_type = match doc_type_str.as_str() {
        "PASSPORT" => IdentityType::Passport,
        "NATIONAL_ID" => IdentityType::NationalId,
        "PHONE" => IdentityType::Phone,
        "EMAIL" => IdentityType::Email,
        _ => IdentityType::Passport,
    };
    VerificationDocument {
        id,
        case_id,
        document_type,
        encrypted_object_ref,
        key_id,
        checksum,
        mime,
        size_bytes,
        uploaded_at,
        uploaded_by,
        retention_until,
        access_class: AccessClass::parse(&access_class_str)
            .unwrap_or(AccessClass::IdentityDocument),
        removed_at,
    }
}

impl CaseRepository for PgCaseRepository {
    #[allow(clippy::type_complexity)]
    fn find_customer<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<(Uuid, Option<Uuid>)>, CaseError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, (Uuid, Option<Uuid>)>(
                "SELECT organization_id, user_id FROM customers WHERE id = $1",
            )
            .bind(customer_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;
            Ok(row)
        })
    }

    fn find_latest_policy<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> BoxFuture<'a, Result<Option<VerificationPolicy>, CaseError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, (Uuid, i32, Vec<String>, i32, bool)>(
                "SELECT organization_id, version, accepted_document_types, documents_required, branch_review_allowed \
                 FROM verification_policies WHERE organization_id = $1 ORDER BY version DESC LIMIT 1",
            )
            .bind(organization_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(row.map(|(org, ver, types, required, branch_allowed)| {
                let accepted = types
                    .into_iter()
                    .filter_map(|t| match t.as_str() {
                        "PASSPORT" => Some(IdentityType::Passport),
                        "NATIONAL_ID" => Some(IdentityType::NationalId),
                        "PHONE" => Some(IdentityType::Phone),
                        "EMAIL" => Some(IdentityType::Email),
                        _ => None,
                    })
                    .collect();
                VerificationPolicy {
                    organization_id: org,
                    version: ver,
                    accepted_document_types: accepted,
                    documents_required: required,
                    branch_review_allowed: branch_allowed,
                }
            }))
        })
    }

    fn find_policy<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        version: i32,
    ) -> BoxFuture<'a, Result<Option<VerificationPolicy>, CaseError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, (Uuid, i32, Vec<String>, i32, bool)>(
                "SELECT organization_id, version, accepted_document_types, documents_required, branch_review_allowed \
                 FROM verification_policies WHERE organization_id = $1 AND version = $2",
            )
            .bind(organization_id)
            .bind(version)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(row.map(|(org, ver, types, required, branch_allowed)| {
                let accepted = types
                    .into_iter()
                    .filter_map(|t| match t.as_str() {
                        "PASSPORT" => Some(IdentityType::Passport),
                        "NATIONAL_ID" => Some(IdentityType::NationalId),
                        "PHONE" => Some(IdentityType::Phone),
                        "EMAIL" => Some(IdentityType::Email),
                        _ => None,
                    })
                    .collect();
                VerificationPolicy {
                    organization_id: org,
                    version: ver,
                    accepted_document_types: accepted,
                    documents_required: required,
                    branch_review_allowed: branch_allowed,
                }
            }))
        })
    }

    fn find_open_case_by_customer<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<VerificationCase>, CaseError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, CaseRow>(
                "SELECT id, organization_id, customer_id, provider, status, policy_version, branch_id, \
                 submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version \
                 FROM verification_cases WHERE customer_id = $1 AND status IN ('DRAFT', 'SUBMITTED', 'UNDER_REVIEW', 'NEEDS_UPDATE') \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(customer_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(row.map(map_case))
        })
    }

    fn insert_case<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case: &'a VerificationCase,
    ) -> BoxFuture<'a, Result<Option<VerificationCase>, CaseError>> {
        Box::pin(async move {
            let now = Utc::now();
            let row = sqlx::query_as::<_, CaseRow>(
                "INSERT INTO verification_cases (id, organization_id, customer_id, provider, status, \
                 policy_version, branch_id, submitted_at, decided_at, expires_at, reviewed_by, \
                 decision_reason_code, version, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14) \
                 ON CONFLICT (customer_id) WHERE status IN ('DRAFT', 'SUBMITTED', 'UNDER_REVIEW', 'NEEDS_UPDATE') DO NOTHING \
                 RETURNING id, organization_id, customer_id, provider, status, policy_version, branch_id, \
                 submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version",
            )
            .bind(case.id)
            .bind(case.organization_id)
            .bind(&case.customer_id)
            .bind(case.provider.as_str())
            .bind(case.status.as_str())
            .bind(case.policy_version)
            .bind(case.branch_id.as_ref().map(|b| b.as_str()))
            .bind(case.submitted_at)
            .bind(case.decided_at)
            .bind(case.expires_at)
            .bind(case.reviewed_by)
            .bind(case.decision_reason.map(|r| r.as_str()))
            .bind(case.version)
            .bind(now)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(row.map(map_case))
        })
    }

    fn find_case_by_id_for_update<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
    ) -> BoxFuture<'a, Result<Option<VerificationCase>, CaseError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, CaseRow>(
                "SELECT id, organization_id, customer_id, provider, status, policy_version, branch_id, \
                 submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version \
                 FROM verification_cases WHERE id = $1 FOR UPDATE",
            )
            .bind(case_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(row.map(map_case))
        })
    }

    fn update_case_submitted<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
        submitted_at: DateTime<Utc>,
        version: i32,
    ) -> BoxFuture<'a, Result<(), CaseError>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE verification_cases \
                 SET status = 'SUBMITTED', submitted_at = $2, reviewed_by = NULL, decided_at = NULL, \
                 decision_reason_code = NULL, version = $3, updated_at = $2 WHERE id = $1",
            )
            .bind(case_id)
            .bind(submitted_at)
            .bind(version)
            .execute(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(())
        })
    }

    fn find_document_by_ref<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_ref: &'a str,
    ) -> BoxFuture<'a, Result<Option<VerificationDocument>, CaseError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, DocRow>(
                "SELECT id, case_id, document_type, encrypted_object_ref, key_id, checksum, \
                 mime, size_bytes, uploaded_at, uploaded_by, retention_until, access_class, removed_at \
                 FROM verification_documents WHERE encrypted_object_ref = $1",
            )
            .bind(object_ref)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(row.map(map_doc))
        })
    }

    fn find_document_by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<VerificationDocument>, CaseError>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, DocRow>(
                "SELECT id, case_id, document_type, encrypted_object_ref, key_id, checksum, \
                 mime, size_bytes, uploaded_at, uploaded_by, retention_until, access_class, removed_at \
                 FROM verification_documents WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(row.map(map_doc))
        })
    }

    fn list_active_documents_by_case<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        case_id: Uuid,
    ) -> BoxFuture<'a, Result<Vec<VerificationDocument>, CaseError>> {
        Box::pin(async move {
            let rows = sqlx::query_as::<_, DocRow>(
                "SELECT id, case_id, document_type, encrypted_object_ref, key_id, checksum, \
                 mime, size_bytes, uploaded_at, uploaded_by, retention_until, access_class, removed_at \
                 FROM verification_documents WHERE case_id = $1 AND removed_at IS NULL AND purged_at IS NULL \
                 ORDER BY uploaded_at ASC",
            )
            .bind(case_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(rows.into_iter().map(map_doc).collect())
        })
    }

    fn insert_document<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        doc: &'a VerificationDocument,
    ) -> BoxFuture<'a, Result<(), CaseError>> {
        Box::pin(async move {
            let now = Utc::now();
            sqlx::query(
                "INSERT INTO verification_documents (id, organization_id, case_id, document_type, \
                 encrypted_object_ref, key_id, checksum, mime, size_bytes, uploaded_at, uploaded_by, \
                 retention_until, access_class, removed_at, purged_at, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, NULL, NULL, $14)",
            )
            .bind(doc.id)
            .bind(organization_id)
            .bind(doc.case_id)
            .bind(doc.document_type.as_str())
            .bind(&doc.encrypted_object_ref)
            .bind(&doc.key_id)
            .bind(&doc.checksum)
            .bind(&doc.mime)
            .bind(doc.size_bytes)
            .bind(doc.uploaded_at)
            .bind(doc.uploaded_by)
            .bind(doc.retention_until)
            .bind(doc.access_class.as_str())
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(())
        })
    }

    fn remove_document<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), CaseError>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE verification_documents SET removed_at = $2, encrypted_object_ref = NULL WHERE id = $1",
            )
            .bind(id)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(|e| CaseError::Store(pos_error::AppError::Database(e)))?;

            Ok(())
        })
    }
}
