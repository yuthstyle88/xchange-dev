//! Organisation-level views (EX-3 #1071; design `executive-finance.md` §7.2, §8.1, D-5): one answer over every
//! branch in the caller's `BranchScope`, polled per branch.
//!
//! An organisation view is mounted twice, on one handler:
//! - its own route (A01: `GET /api/executive/overview`), naming branches with `branches=B1,B2`. An empty item is
//!   `400 validationFailed`; any id outside the caller's scope, existing or not, is `403 outOfScope` (ruling on
//!   #1071 gap 7). A caller whose roles do not qualify is `403 outOfScope` too (§8, §9);
//! - `/api/v3/views/<NAME>`, because every file in `views/` is in `VIEWS` and answers a branch probe there. It names
//!   one `branch`, and refuses in this order: the policy's own refusal; `400 noSuchBranch` for any id that is not a
//!   branch of the caller's organisation, whether it exists elsewhere or not; `403 outOfScope` for one of the
//!   organisation's branches outside the caller's scope. Otherwise it answers for that branch (lane controller's
//!   ruling on #1071, issuecomment-6108432223).

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use pos_error::AppResult;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_rate::domain::validation::NO_SUCH_BRANCH;

use super::error::V3Error;
use crate::bootstrap::AppState;
use crate::presentation::dto::v3::executive_overview::OrgViewAnswer;
use crate::presentation::pages;

/// An organisation view's own query keys, read from the first value of each key. `branches`, `branch` and
/// `versions` are the seam's.
pub trait OrgFilter: Sized + Send + Sync + 'static {
    fn parse(query: &BTreeMap<String, String>) -> Result<Self, V3Error>;
}

pub trait OrgView: Send + Sync + 'static {
    const NAME: &'static str;
    const INHERITS: (EntryPath, &'static str);
    type Filter: OrgFilter;
    type Data: serde::Serialize + ts_rs::TS + Send;

    /// The current version of every branch in `scope`.
    fn versions_in(
        &self,
        scope: &BranchScope,
    ) -> impl Future<Output = AppResult<BTreeMap<BranchId, i64>>> + Send;

    /// The answer over `scope`, with the per-branch parts of `changed` only.
    fn load(
        &self,
        scope: &BranchScope,
        filter: &Self::Filter,
        changed: &[BranchId],
    ) -> impl Future<Output = AppResult<Self::Data>> + Send;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mount {
    Org,
    Views,
}

pub struct OrgViewEndpoint;

impl OrgViewEndpoint {
    /// Serves `view` at `path` and at `/api/v3/views/<NAME>`, both through the same handler.
    pub fn register<V: OrgView>(cfg: &mut web::ServiceConfig, path: &str, view: Arc<V>) {
        let org = view.clone();
        cfg.route(
            path,
            web::get().to(move |req: HttpRequest| {
                let view = org.clone();
                async move { handle::<V>(&req, &view, Mount::Org).await }
            }),
        );
        cfg.route(
            &format!("/api/v3/views/{}", V::NAME),
            web::get().to(move |req: HttpRequest| {
                let view = view.clone();
                async move { handle::<V>(&req, &view, Mount::Views).await }
            }),
        );
    }
}

/// `400 validationFailed` on `field`.
pub fn validation_failed(field: &'static str) -> V3Error {
    V3Error::new(
        "validationFailed",
        Some(field),
        "ข้อมูลไม่ถูกต้อง",
        StatusCode::BAD_REQUEST,
    )
}

fn server_error() -> V3Error {
    V3Error::new(
        "serverError",
        None,
        "เซิร์ฟเวอร์ขัดข้อง",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
}

async fn handle<V: OrgView>(req: &HttpRequest, view: &V, mount: Mount) -> HttpResponse {
    match answer::<V>(req, view, mount).await {
        Ok(resp) => resp,
        Err(e) => e.error_response(),
    }
}

async fn answer<V: OrgView>(
    req: &HttpRequest,
    view: &V,
    mount: Mount,
) -> Result<HttpResponse, V3Error> {
    let now = chrono::Utc::now();
    let action = Action::new(V::INHERITS.1);
    let callers = pages::get_callers(req).await;
    let user = callers.user.as_ref();
    if let Err(e) = Policy::authorize(user, V::INHERITS.0, &action, None, now) {
        return Err(match (mount, e) {
            (Mount::Org, PolicyError::RoleNotAllowedHere | PolicyError::RoleNotAllowed) => {
                PolicyError::OutOfScope.into()
            }
            (_, e) => e.into(),
        });
    }
    let user = user.ok_or_else(|| V3Error::from(PolicyError::SignIn))?;

    let query = query_map(req)?;
    let permission = match Policy::need(V::INHERITS.0, &action) {
        Some(Need::Permission(p)) => p,
        _ => return Err(server_error()),
    };
    let scope = BranchScope::of(user, permission, now);

    let pool = req
        .app_data::<web::Data<AppState>>()
        .map(|s| s.db.clone())
        .ok_or_else(server_error)?;
    let organisation: Vec<String> =
        sqlx::query_scalar("SELECT id FROM branches WHERE organization_id::text = $1 ORDER BY id")
            .bind(&user.organization_id)
            .fetch_all(&pool)
            .await
            .map_err(|_| server_error())?;
    let in_scope: Vec<BranchId> = organisation
        .iter()
        .map(BranchId::new)
        .filter(|b| scope.contains(b))
        .collect();

    let branches = match mount {
        Mount::Org => match query.get("branches") {
            None => in_scope,
            Some(list) => {
                let mut named = Vec::new();
                for item in list.split(',') {
                    if item.is_empty() {
                        return Err(validation_failed("branches"));
                    }
                    let b = BranchId::new(item);
                    if !in_scope.contains(&b) {
                        return Err(PolicyError::OutOfScope.into());
                    }
                    if !named.contains(&b) {
                        named.push(b);
                    }
                }
                named.sort();
                named
            }
        },
        Mount::Views => match query.get("branch").filter(|b| !b.is_empty()) {
            None => in_scope,
            Some(b) => {
                if !organisation.iter().any(|o| o == b) {
                    return Err(V3Error::new(
                        "noSuchBranch",
                        Some("branch"),
                        NO_SUCH_BRANCH,
                        StatusCode::BAD_REQUEST,
                    ));
                }
                let b = BranchId::new(b.as_str());
                if !in_scope.contains(&b) {
                    return Err(PolicyError::OutOfScope.into());
                }
                vec![b]
            }
        },
    };

    let filter = V::Filter::parse(&query)?;
    let held: Option<BTreeMap<String, i64>> = match query.get("versions") {
        None => None,
        Some(text) => Some(serde_json::from_str(text).map_err(|_| validation_failed("versions"))?),
    };

    let scope = BranchScope::Branches(branches.clone());
    let current = view.versions_in(&scope).await.map_err(|_| server_error())?;
    let changed: Vec<BranchId> = branches
        .iter()
        // a branch with no version row has never changed: unchanged while the client holds none for it either
        .filter(|b| match &held {
            None => true,
            Some(held) => held.get(b.as_str()) != current.get(*b),
        })
        .cloned()
        .collect();
    let versions: BTreeMap<String, i64> = current
        .into_iter()
        .map(|(b, v)| (b.as_str().to_owned(), v))
        .collect();

    let answer = if held.is_some() && changed.is_empty() {
        OrgViewAnswer::<V::Data> {
            changed: false,
            versions,
            data: None,
        }
    } else {
        let data = view
            .load(&scope, &filter, &changed)
            .await
            .map_err(|_| server_error())?;
        OrgViewAnswer {
            changed: true,
            versions,
            data: Some(data),
        }
    };
    Ok(HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(answer))
}

/// The query's keys, each with its first value.
fn query_map(req: &HttpRequest) -> Result<BTreeMap<String, String>, V3Error> {
    let pairs = web::Query::<Vec<(String, String)>>::from_query(req.query_string())
        .map_err(|_| validation_failed("query"))?;
    let mut map = BTreeMap::new();
    for (k, v) in pairs.into_inner() {
        map.entry(k).or_insert(v);
    }
    Ok(map)
}
