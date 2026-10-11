//! The `handover_receivers` view (CA-8 #624; lane controller's ruling issuecomment-6107853376): the active users of
//! the session branch's organisation who hold `ShiftOperate` at the session branch, the caller excluded, ordered by name and user id. A valid receiver
//! is exactly a person who could later accept (`PersonRules::receiver_verifies_handover`). No clocked-in filter: the
//! server requires none. Authorised like `handoverStart` (`ShiftOperate`, the same row).

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::{
    Actor as User, Authorizer, Permission, Role, ScopedRole, Target,
};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::handover_receivers::{HandoverReceiverRow, HandoverReceivers};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct HandoverReceiversView {
    pool: PgPool,
}

/// A candidate's valid role: user, name, organisation, role code, branch, valid from, valid to.
type CandidateRole = (
    String,
    String,
    String,
    String,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

impl BranchView for HandoverReceiversView {
    const NAME: &'static str = "handover_receivers";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoverStart");
    type Data = HandoverReceivers;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let me = who.id().to_string();
        async move {
            let now = Utc::now();
            // the session branch's organisation: only its people can receive here (PR 1325 review, issuecomment-6108602501)
            let session_org: Option<String> =
                sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&pool)
                    .await
                    .map_err(AppError::Database)?;
            let Some(session_org) = session_org else {
                return Ok(HandoverReceivers { rows: Vec::new() });
            };
            // every active user of that organisation (but the caller) with a role of that organisation valid now that
            // reaches this branch: one scoped to it, or organisation-wide
            let rows: Vec<CandidateRole> = sqlx::query_as(
                "SELECT u.id::text, COALESCE(u.name, ''), u.organization_id::text, r.role_code, r.branch_id, r.valid_from, r.valid_to \
                 FROM users u JOIN user_roles r ON r.user_id = u.id \
                 WHERE u.status = 'ACTIVE' AND u.id::text <> $1 \
                   AND u.organization_id::text = $4 AND r.organization_id::text = $4 \
                   AND (r.branch_id = $2 OR r.branch_id IS NULL) \
                   AND r.valid_from <= $3 AND (r.valid_to IS NULL OR r.valid_to > $3)",
            )
            .bind(&me)
            .bind(branch.as_str())
            .bind(now)
            .bind(&session_org)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let mut people: BTreeMap<String, (String, String, Vec<ScopedRole>)> = BTreeMap::new();
            for (id, name, org, code, role_branch, from, to) in rows {
                let entry = people.entry(id).or_insert_with(|| (name, org, Vec::new()));
                if let Some(role) = Role::parse(&code) {
                    entry
                        .2
                        .push(ScopedRole::new(role, role_branch.map(BranchId::new), from, to));
                }
            }

            // holds ShiftOperate at this branch, as the Authorizer decides it
            let mut receivers: Vec<HandoverReceiverRow> = people
                .into_iter()
                .filter(|(id, (_, org, roles))| {
                    let user = User::new(id, org, roles.clone(), None);
                    // the target is the session branch in the session's organisation, never the candidate's own
                    let at = Target::new(&session_org, Some(branch.clone()), None);
                    Authorizer::check(Some(&user), Permission::ShiftOperate, &at, now).is_ok()
                })
                .map(|(user_id, (name, _, _))| HandoverReceiverRow { user_id, name })
                .collect();
            receivers.sort_by(|a, b| (&a.name, &a.user_id).cmp(&(&b.name, &b.user_id)));
            Ok(HandoverReceivers { rows: receivers })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(HandoverReceiversView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    HandoverReceiverRow::export_all(&cfg)?;
    HandoverReceivers::export_all(&cfg)?;
    Ok(())
}
