//! Ticket #4023, behaviour row 5 — the `approval_requests` view (#2694) lists an organisation-wide request
//! to the organisation that asked for it, and not to another. Written before the implementation by the
//! ticket's controller (claude/studio-claude-7). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `approval_requests_org_scope_test.rs` inserts `organization_id` by hand, so it never exercises the
//! writer. Here the request goes through `ApprovalService::request` and `PgApprovalRepository`, and the
//! view reads what the writer stored. Before #4023 the insert never wrote `organization_id`, so every
//! request took the default organisation: organisation B saw none of its own and organisation A saw B's.
//!
//! Rows 1–4 are `api/crates/xc-approval/tests/approval_request_organisation_test.rs`.

mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use quote_common::TestDb;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::presentation::dto::v3::approval_requests::ApprovalRequestsFilter;
use xchange::presentation::http::v3::approval_requests::ApprovalRequestsBranchView;
use xchange::presentation::http::v3::view::BranchView;

const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
const ORG_B: &str = "00000000-0000-0000-0000-000000000099";
/// A type the view's registry knows, decided by any EXECUTIVE (`xc-rate` `RatePublicationPolicy`).
const TYPE: &str = "RATE_PUBLICATION";

type Boxed<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

/// Stands in for the rate publication, so the request opens without a real publication.
struct Subject;

impl ApprovalSubject for Subject {
    fn type_name(&self) -> &'static str {
        TYPE
    }

    fn version_hash<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
    ) -> Boxed<'a, String> {
        Box::pin(async { Ok("h".to_owned()) })
    }

    fn on_decided<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
        _outcome: Outcome,
    ) -> Boxed<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

struct Anyone;

impl ApprovalPolicy for Anyone {
    fn may_decide(&self, _who: &Actor, _request: &ApprovalRequest) -> Result<(), Refusal> {
        Ok(())
    }

    fn policy_version(&self) -> String {
        "1".to_owned()
    }
}

/// An executive of `organization`: EXECUTIVE + ADMIN, organisation-wide, so the request stores no branch.
fn executive(id: &str, organization: &str) -> Actor {
    Actor::new(
        id,
        organization,
        [Role::Executive, Role::Admin]
            .into_iter()
            .map(|role| ScopedRole::new(role, None, DateTime::<Utc>::UNIX_EPOCH, None))
            .collect(),
        None,
    )
}

async fn setup(db: &TestDb) {
    db.exec(&format!(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ('{ORG_B}', 'ORG2', 'Asia/Bangkok', now())"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO branches (id, name, organization_id, code) VALUES \
         ('XC-B01', 'B01', '{ORG_A}', 'XC-B01'), \
         ('ORG2-B01', 'ORG2 Branch', '{ORG_B}', 'ORG2-B01')"
    ))
    .await;
}

/// `by` opens a request through the real writer.
async fn open(db: &TestDb, by: &Actor, object_id: &str) -> ApprovalRequest {
    let mut registry = ApprovalRegistry::new();
    registry.register(Arc::new(Subject), Arc::new(Anyone));
    let repository: Arc<dyn ApprovalRepository> =
        Arc::new(PgApprovalRepository::new(db.pool.clone()));
    let audit: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(db.pool.clone()));
    let svc = ApprovalService::new(registry, repository, audit);
    let mut tx = db.pool.begin().await.expect("begin");
    let request = svc
        .request(
            &mut tx,
            TYPE,
            object_id,
            &json!({ "n": 1 }),
            by,
            &format!("key-{object_id}"),
        )
        .await
        .expect("the request opens");
    tx.commit().await.expect("commit");
    request
}

/// The ids the view lists at `branch` for `who`.
async fn listed(db: &TestDb, branch: &str, who: &Actor) -> Vec<String> {
    let view = ApprovalRequestsBranchView::new(db.pool.clone());
    let data = view
        .load(
            &BranchId::new(branch),
            who,
            &ApprovalRequestsFilter::default(),
        )
        .await
        .expect("the view loads");
    data.requests.into_iter().map(|r| r.id).collect()
}

#[tokio::test]
async fn row5_the_view_lists_an_organisation_wide_request_to_its_own_organisation_only() {
    let db = TestDb::start().await;
    setup(&db).await;
    let exec_a = executive("exec-a", ORG_A);
    let exec_b = executive("exec-b", ORG_B);
    let of_b = open(&db, &exec_b, "obj-b").await;
    let of_a = open(&db, &exec_a, "obj-a").await;

    assert_eq!(
        listed(&db, "ORG2-B01", &exec_b).await,
        vec![of_b.id.to_string()],
        "organisation B's view lists B's own organisation-wide request, and not A's"
    );
    assert_eq!(
        listed(&db, "XC-B01", &exec_a).await,
        vec![of_a.id.to_string()],
        "organisation A's view lists A's own organisation-wide request, and not B's"
    );
    db.finish().await;
}
