//! v3 View traits and endpoint registration (§7.2).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use pos_error::AppResult;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;

use super::error::V3Error;
use crate::bootstrap::AppState;
pub use crate::presentation::dto::v3::common::ViewAnswer;
use crate::presentation::pages::SessionReader;

pub type RowId = String;

pub trait BranchView: Send + Sync + 'static {
    const NAME: &'static str;
    const INHERITS: (EntryPath, &'static str);
    type Data: serde::Serialize + ts_rs::TS + Send;
    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send;
}

pub trait PagedView: Send + Sync + 'static {
    const NAME: &'static str;
    const INHERITS: (EntryPath, &'static str);
    type Row: serde::Serialize + ts_rs::TS + Send;
    fn page(
        &self,
        branch: &BranchId,
        who: &Actor,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send;
}

#[derive(Debug, Default)]
pub struct ViewQuery {
    pub branch: Option<String>,
    pub since: Option<i64>,
    pub before: Option<RowId>,
    pub limit: Option<u16>,
}

pub struct ViewEndpoint;

impl ViewEndpoint {
    pub fn register<V: BranchView>(cfg: &mut web::ServiceConfig, view: Arc<V>) {
        let path = format!("/api/v3/views/{}", V::NAME);
        cfg.route(
            &path,
            web::get().to(move |req: HttpRequest| {
                let view = view.clone();
                async move { handle_branch_view::<V>(&req, &view).await }
            }),
        );
    }

    pub fn register_paged<V: PagedView>(cfg: &mut web::ServiceConfig, view: Arc<V>) {
        let path = format!("/api/v3/views/{}", V::NAME);
        cfg.route(
            &path,
            web::get().to(move |req: HttpRequest| {
                let view = view.clone();
                async move { handle_paged_view::<V>(&req, &view).await }
            }),
        );
    }
}

struct ParsedQuery {
    named_branch: Option<BranchId>,
    since: Option<i64>,
    before: Option<RowId>,
    limit: Option<u16>,
}

fn parse_query(req: &HttpRequest) -> Result<ParsedQuery, V3Error> {
    let query_str = req.query_string();
    if query_str.is_empty() {
        return Ok(ParsedQuery {
            named_branch: None,
            since: None,
            before: None,
            limit: None,
        });
    }

    let pairs = web::Query::<Vec<(String, String)>>::from_query(query_str).map_err(|_| {
        V3Error::new(
            "invalidRequest",
            None,
            "คำขอไม่ถูกต้อง",
            StatusCode::BAD_REQUEST,
        )
    })?;

    let mut map = HashMap::new();
    for (k, v) in pairs.into_inner() {
        map.entry(k).or_insert(v);
    }

    let named_branch = match map.get("branch") {
        None => None,
        Some(b) if b.is_empty() => None,
        Some(b) => {
            if !BranchId::is_well_formed(b) {
                return Err(V3Error::new(
                    "invalidRequest",
                    None,
                    "คำขอไม่ถูกต้อง",
                    StatusCode::BAD_REQUEST,
                ));
            }
            Some(BranchId::new(b))
        }
    };

    let since = match map.get("since") {
        None => None,
        Some(s) => match s.parse::<i64>() {
            Ok(v) if v >= 0 => Some(v),
            _ => {
                return Err(V3Error::new(
                    "invalidRequest",
                    None,
                    "คำขอไม่ถูกต้อง",
                    StatusCode::BAD_REQUEST,
                ));
            }
        },
    };

    let limit = match map.get("limit") {
        None => None,
        Some(l) => match l.parse::<u16>() {
            Ok(v) if (1..=100).contains(&v) => Some(v),
            _ => {
                return Err(V3Error::new(
                    "invalidRequest",
                    None,
                    "คำขอไม่ถูกต้อง",
                    StatusCode::BAD_REQUEST,
                ));
            }
        },
    };

    let before = match map.get("before") {
        None => None,
        Some(b) if b.is_empty() => None,
        Some(b) => Some(b.clone()),
    };

    Ok(ParsedQuery {
        named_branch,
        since,
        before,
        limit,
    })
}

async fn authenticate_and_authorize(
    req: &HttpRequest,
    inherits: (EntryPath, &'static str),
) -> Result<Actor, HttpResponse> {
    // 1. Sessions from SessionReader
    let actor = if let Some(reader) = req.app_data::<web::Data<dyn SessionReader>>() {
        let sessions = reader.sessions(req).await;
        // 2. The actor by entry path
        match inherits.0 {
            EntryPath::StaffRpc => sessions.staff,
            _ => sessions.owner.or(sessions.staff),
        }
    } else {
        None
    };

    // 3. Policy check
    let action = Action::new(inherits.1);
    if let Err(e) = Policy::allow(actor.as_ref(), inherits.0, &action) {
        return Err(V3Error::from(e).error_response());
    }

    actor.ok_or_else(|| V3Error::from(PolicyError::SignIn).error_response())
}

async fn handle_branch_view<V: BranchView>(req: &HttpRequest, view: &V) -> HttpResponse {
    let who = match authenticate_and_authorize(req, V::INHERITS).await {
        Ok(who) => who,
        Err(resp) => return resp,
    };

    // 4. Parse query
    let query = match parse_query(req) {
        Ok(q) => q,
        Err(e) => return e.error_response(),
    };

    // 5. Branch
    let branch = Policy::branch(Some(&who), query.named_branch.as_ref());

    // 6. BranchVersions
    let pool = match req.app_data::<web::Data<AppState>>() {
        Some(state) => state.db.clone(),
        None => {
            return V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    let versions = BranchVersions::new(pool);
    let version = match versions.current(&branch).await {
        Ok(v) => v,
        Err(_) => {
            return V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    // 7. Check since
    if query.since == Some(version) {
        let answer = ViewAnswer::<V::Data> {
            version,
            changed: false,
            data: None,
        };
        return HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(answer);
    }

    match view.load(&branch, &who).await {
        Ok(data) => {
            let answer = ViewAnswer {
                version,
                changed: true,
                data: Some(data),
            };
            HttpResponse::Ok()
                .insert_header((header::CONTENT_TYPE, "application/json"))
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(answer)
        }
        Err(_) => V3Error::new(
            "serverError",
            None,
            "เซิร์ฟเวอร์ขัดข้อง",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .error_response(),
    }
}

async fn handle_paged_view<V: PagedView>(req: &HttpRequest, view: &V) -> HttpResponse {
    let who = match authenticate_and_authorize(req, V::INHERITS).await {
        Ok(who) => who,
        Err(resp) => return resp,
    };

    // 4. Parse query
    let query = match parse_query(req) {
        Ok(q) => q,
        Err(e) => return e.error_response(),
    };

    // 5. Branch
    let branch = Policy::branch(Some(&who), query.named_branch.as_ref());

    // 6. BranchVersions
    let pool = match req.app_data::<web::Data<AppState>>() {
        Some(state) => state.db.clone(),
        None => {
            return V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    let versions = BranchVersions::new(pool);
    let version = match versions.current(&branch).await {
        Ok(v) => v,
        Err(_) => {
            return V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    // 7. Check since (only when before is None)
    if query.before.is_none() && query.since == Some(version) {
        let answer = ViewAnswer::<Vec<V::Row>> {
            version,
            changed: false,
            data: None,
        };
        return HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(answer);
    }

    let limit = query.limit.unwrap_or(50);
    match view.page(&branch, &who, query.before, limit).await {
        Ok(rows) => {
            let answer = ViewAnswer {
                version,
                changed: true,
                data: Some(rows),
            };
            HttpResponse::Ok()
                .insert_header((header::CONTENT_TYPE, "application/json"))
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(answer)
        }
        Err(_) => V3Error::new(
            "serverError",
            None,
            "เซิร์ฟเวอร์ขัดข้อง",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .error_response(),
    }
}
