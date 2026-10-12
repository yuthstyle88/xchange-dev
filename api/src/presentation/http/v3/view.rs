//! v3 View traits and endpoint registration (§7.2).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xc_rate::domain::validation::NO_SUCH_BRANCH;

use super::caller::Caller;
use super::error::V3Error;
use super::export_gate::ExportGate;
use crate::bootstrap::AppState;
pub use crate::presentation::dto::v3::common::ViewAnswer;
use crate::presentation::pages;

pub type RowId = String;

pub trait ViewFilter:
    serde::de::DeserializeOwned + ts_rs::TS + Default + Send + Sync + 'static
{
    fn check(&self) -> Result<(), V3Error> {
        Ok(())
    }
}

pub trait BranchView: Send + Sync + 'static {
    const NAME: &'static str;
    const INHERITS: (EntryPath, &'static str);
    const VERSIONED: bool = true;
    /// A version that is not the branch's (design §12.16). None = use branch_versions.
    fn own_version(
        &self,
        _branch: &BranchId,
        _who: &Actor,
    ) -> impl Future<Output = AppResult<Option<i64>>> + Send {
        async { Ok(None) }
    }
    type Data: serde::Serialize + ts_rs::TS + Send;
    type Filter: ViewFilter;
    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send;
    /// APP-18 #1373 (option B): `load` with the seam's [`Caller`]. By default it calls `load` with `caller.user`, and
    /// answers `signIn` when there is none; a view that serves an anonymous or app-session caller overrides it.
    fn load_as(
        &self,
        branch: &BranchId,
        caller: &Caller,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        async move {
            match caller.user.as_ref() {
                Some(who) => self.load(branch, who, filter).await,
                None => Err(AppError::Unauthenticated),
            }
        }
    }
    /// The refusal this view answers for a `load` error, instead of the core's 500 (#1349 ruling A).
    /// None = the core's answer.
    fn refusal(&self, _err: &AppError) -> Option<V3Error> {
        None
    }
}

pub trait PagedView: Send + Sync + 'static {
    const NAME: &'static str;
    const INHERITS: (EntryPath, &'static str);
    type Row: serde::Serialize + ts_rs::TS + Send;
    type Filter: ViewFilter;
    fn page(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send;
    /// APP-18 #1373 (option B): `page` with the seam's [`Caller`]; the default calls `page` with `caller.user` and
    /// answers `signIn` when there is none.
    fn page_as(
        &self,
        branch: &BranchId,
        caller: &Caller,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        async move {
            match caller.user.as_ref() {
                Some(who) => self.page(branch, who, filter, before, limit).await,
                None => Err(AppError::Unauthenticated),
            }
        }
    }
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

/// Why a view is read: `purpose=export` marks an export (EX-9 #1147). Its only value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Export,
}

/// The query keys the core reads itself; never a filter key, never in an export's audited filter.
const CORE_KEYS: [&str; 5] = ["branch", "since", "before", "limit", "purpose"];

struct ParsedQuery {
    named_branch: Option<BranchId>,
    since: Option<i64>,
    before: Option<RowId>,
    limit: Option<u16>,
    purpose: Option<Purpose>,
}

fn invalid_purpose() -> V3Error {
    V3Error::new(
        "invalidRequest",
        Some("purpose"),
        "คำขอไม่ถูกต้อง",
        StatusCode::BAD_REQUEST,
    )
}

fn parse_query(req: &HttpRequest) -> Result<ParsedQuery, V3Error> {
    let query_str = req.query_string();
    if query_str.is_empty() {
        return Ok(ParsedQuery {
            named_branch: None,
            since: None,
            before: None,
            limit: None,
            purpose: None,
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
    let mut purpose = None;
    for (k, v) in pairs.into_inner() {
        // Every `purpose` is read, not only the first: `export` is its one value (EX-9 #1147).
        if k == "purpose" {
            if v != "export" {
                return Err(invalid_purpose());
            }
            purpose = Some(Purpose::Export);
        }
        map.entry(k).or_insert(v);
    }

    let named_branch = parse_branch(map.get("branch").map(|s| s.as_str())).map_err(|()| {
        V3Error::new(
            "invalidRequest",
            None,
            "คำขอไม่ถูกต้อง",
            StatusCode::BAD_REQUEST,
        )
    })?;

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
        purpose,
    })
}

/// An export's filter for its audit row (design §8.4): every query key but the core's, as decoded strings, the
/// first value of each. The audit log passes it through the shared `Redactor`.
fn export_filter(req: &HttpRequest) -> serde_json::Value {
    let mut filter = serde_json::Map::new();
    if let Ok(pairs) = web::Query::<Vec<(String, String)>>::from_query(req.query_string()) {
        for (k, v) in pairs.into_inner() {
            if !CORE_KEYS.contains(&k.as_str()) && !filter.contains_key(&k) {
                filter.insert(k, serde_json::Value::String(v));
            }
        }
    }
    serde_json::Value::Object(filter)
}

fn decode_key(raw_key: &str) -> String {
    match web::Query::<Vec<(String, String)>>::from_query(&format!("{raw_key}=")) {
        Ok(pairs) => pairs
            .into_inner()
            .into_iter()
            .next()
            .map(|(k, _)| k)
            .unwrap_or_else(|| raw_key.to_string()),
        Err(_) => raw_key.to_string(),
    }
}

struct FieldInspector<'a>(&'a mut Option<&'static [&'static str]>);

impl<'de, 'a> serde::de::Deserializer<'de> for FieldInspector<'a> {
    type Error = serde::de::value::Error;

    fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        Err(serde::de::Error::custom("not struct"))
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        *self.0 = Some(fields);
        Err(serde::de::Error::custom("captured"))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

pub fn struct_fields<F: serde::de::DeserializeOwned>() -> Option<&'static [&'static str]> {
    let mut fields = None;
    let _ = F::deserialize(FieldInspector(&mut fields));
    fields
}

fn parse_filter<F: ViewFilter>(req: &HttpRequest) -> Result<F, V3Error> {
    let query_str = req.query_string();
    if query_str.is_empty() {
        let filter = F::default();
        filter.check()?;
        return Ok(filter);
    }

    let mut seen_keys = std::collections::HashSet::new();
    let mut filter_segments = Vec::new();

    for segment in query_str.split('&') {
        if segment.is_empty() {
            continue;
        }
        let raw_key = match segment.split_once('=') {
            Some((k, _)) => k,
            None => segment,
        };
        let key_decoded = decode_key(raw_key);

        match key_decoded.as_str() {
            key if CORE_KEYS.contains(&key) => {
                continue;
            }
            _ => {
                if seen_keys.insert(key_decoded.clone()) {
                    filter_segments.push((key_decoded, segment));
                }
            }
        }
    }

    if filter_segments.is_empty() {
        let filter = F::default();
        filter.check()?;
        return Ok(filter);
    }

    let filter_query = filter_segments
        .iter()
        .map(|(_, seg)| *seg)
        .collect::<Vec<_>>()
        .join("&");

    match web::Query::<F>::from_query(&filter_query) {
        Ok(q) => {
            let filter = q.into_inner();
            filter.check()?;
            Ok(filter)
        }
        Err(_) => {
            let known_fields = struct_fields::<F>().unwrap_or(&[]);
            for (wire_key, seg) in filter_segments {
                if web::Query::<F>::from_query(seg).is_err() {
                    let static_field = known_fields.iter().copied().find(|&f| f == wire_key);
                    return Err(V3Error::new(
                        "invalidRequest",
                        static_field,
                        "คำขอไม่ถูกต้อง",
                        StatusCode::BAD_REQUEST,
                    ));
                }
            }
            Err(V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ))
        }
    }
}

/// Parses the branch query or field according to §12.20.3 step 4.
pub(super) fn parse_branch(raw: Option<&str>) -> Result<Option<BranchId>, ()> {
    match raw {
        None | Some("") => Ok(None),
        Some(b) => {
            if !BranchId::is_well_formed(b) {
                Err(())
            } else {
                Ok(Some(BranchId::new(b)))
            }
        }
    }
}

/// What a view's `load`/`page` fails with when an OR-7 bridge call (`legacy_actor`) finds no v2 actor for the caller
/// (#1030 §3.8). The seam answers it as a refused route does: `403 roleNotAllowedHere`.
pub(crate) const BRIDGE_REFUSED: &str = "OR-7 bridge: the caller maps to no v2 actor";

/// The error a view returns for [`BRIDGE_REFUSED`].
pub(crate) fn bridge_refused() -> AppError {
    AppError::Domain(BRIDGE_REFUSED.to_owned())
}

pub(crate) async fn authenticate_and_authorize(
    req: &HttpRequest,
    inherits: (EntryPath, &'static str),
) -> Result<Actor, HttpResponse> {
    let callers = pages::get_callers(req).await;
    let now = chrono::Utc::now();
    let action = Action::new(inherits.1);

    let user = match inherits.0 {
        EntryPath::StaffRpc => callers.staff_user.as_ref(),
        _ => callers.user.as_ref(),
    };

    let user = match user {
        Some(u) => {
            if let Err(e) = Policy::authorize(Some(u), inherits.0, &action, None, now) {
                return Err(V3Error::from(e).error_response());
            }
            u
        }
        None => {
            let err = Policy::authorize(None, inherits.0, &action, None, now)
                .err()
                .unwrap_or(PolicyError::SignIn);
            return Err(V3Error::from(err).error_response());
        }
    };

    Ok(user.clone())
}

/// 400 noSuchBranch when `branch` is not in `branches`; 500 serverError when it cannot be read.
pub(super) async fn require_branch(req: &HttpRequest, branch: &BranchId) -> Result<(), V3Error> {
    let pool = match req.app_data::<web::Data<AppState>>() {
        Some(state) => state.db.clone(),
        None => {
            return Err(V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };
    let branch_tx = BranchTx::new(Transactor::new(pool));
    let service = BranchService::new(branch_tx);
    match service.require(branch).await {
        Ok(_) => Ok(()),
        Err(AppError::Domain(m)) if m == NO_SUCH_BRANCH => Err(V3Error::new(
            "noSuchBranch",
            Some("branch"),
            NO_SUCH_BRANCH,
            StatusCode::BAD_REQUEST,
        )),
        Err(_) => Err(V3Error::new(
            "serverError",
            None,
            "เซิร์ฟเวอร์ขัดข้อง",
            StatusCode::INTERNAL_SERVER_ERROR,
        )),
    }
}

pub(crate) use super::caller::authenticate_caller;

async fn get_branch_version(req: &HttpRequest, branch: &BranchId) -> Result<i64, V3Error> {
    let pool = match req.app_data::<web::Data<AppState>>() {
        Some(state) => state.db.clone(),
        None => {
            return Err(V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };
    let versions = BranchVersions::new(pool);
    versions.current(branch).await.map_err(|_| {
        V3Error::new(
            "serverError",
            None,
            "เซิร์ฟเวอร์ขัดข้อง",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
    })
}

async fn handle_branch_view<V: BranchView>(req: &HttpRequest, view: &V) -> HttpResponse {
    let caller = match authenticate_caller(req, V::INHERITS).await {
        Ok(res) => res,
        Err(resp) => return resp,
    };

    let query = match parse_query(req) {
        Ok(q) => q,
        Err(e) => return e.error_response(),
    };
    // Only a paged view exports (EX-9 #1147).
    if query.purpose.is_some() {
        return invalid_purpose().error_response();
    }

    let now = chrono::Utc::now();
    let action = Action::new(V::INHERITS.1);
    let grant = match Policy::authorize(
        caller.user.as_ref(),
        V::INHERITS.0,
        &action,
        query.named_branch.as_ref(),
        now,
    ) {
        Ok(g) => g,
        Err(e) => return V3Error::from(e).error_response(),
    };
    let branch = grant.branch;

    if let Err(e) = require_branch(req, &branch).await {
        return e.error_response();
    }

    let filter = match parse_filter::<V::Filter>(req) {
        Ok(f) => f,
        Err(e) => return e.error_response(),
    };

    let version = if let Some(who) = caller.user.as_ref() {
        match view.own_version(&branch, who).await {
            Ok(Some(own_ver)) => own_ver,
            Ok(None) => match get_branch_version(req, &branch).await {
                Ok(v) => v,
                Err(e) => return e.error_response(),
            },
            Err(_) => {
                return V3Error::new(
                    "serverError",
                    None,
                    "เซิร์ฟเวอร์ขัดข้อง",
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .error_response();
            }
        }
    } else {
        match get_branch_version(req, &branch).await {
            Ok(v) => v,
            Err(e) => return e.error_response(),
        }
    };

    if V::VERSIONED && query.since == Some(version) {
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

    match view.load_as(&branch, &caller, &filter).await {
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
        Err(AppError::Unauthenticated) => V3Error::from(PolicyError::SignIn).error_response(),
        Err(AppError::Domain(m)) if m == BRIDGE_REFUSED => {
            V3Error::from(PolicyError::RoleNotAllowedHere).error_response()
        }
        Err(err) => match view.refusal(&err) {
            Some(refusal) => refusal.error_response(),
            None => V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response(),
        },
    }
}

async fn handle_paged_view<V: PagedView>(req: &HttpRequest, view: &V) -> HttpResponse {
    let caller = match authenticate_caller(req, V::INHERITS).await {
        Ok(res) => res,
        Err(resp) => return resp,
    };

    let query = match parse_query(req) {
        Ok(q) => q,
        Err(e) => return e.error_response(),
    };

    let now = chrono::Utc::now();
    let action = Action::new(V::INHERITS.1);
    let grant = match Policy::authorize(
        caller.user.as_ref(),
        V::INHERITS.0,
        &action,
        query.named_branch.as_ref(),
        now,
    ) {
        Ok(g) => g,
        Err(e) => return V3Error::from(e).error_response(),
    };
    let branch = grant.branch;

    // EX-9 (#1147): an export needs `ReportsExport` at the branch the view resolved, before any row is read.
    let export = match query.purpose {
        Some(Purpose::Export) => {
            let Some(ref who) = caller.user else {
                return V3Error::from(PolicyError::SignIn).error_response();
            };
            match ExportGate::check(who, &branch, now) {
                Ok(grant) => Some(grant),
                Err(e) => return e.error_response(),
            }
        }
        None => None,
    };

    if let Err(e) = require_branch(req, &branch).await {
        return e.error_response();
    }

    let filter = match parse_filter::<V::Filter>(req) {
        Ok(f) => f,
        Err(e) => return e.error_response(),
    };

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

    let versions = BranchVersions::new(pool.clone());
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
    match view
        .page_as(&branch, &caller, &filter, query.before, limit)
        .await
    {
        Ok(rows) => {
            // One audit row per answered export request; no audit, no rows (fail closed).
            if let Some(grant) = &export
                && let Err(e) =
                    ExportGate::record(&pool, grant, V::NAME, export_filter(req), rows.len()).await
            {
                return e.error_response();
            }
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
        Err(AppError::Unauthenticated) => V3Error::from(PolicyError::SignIn).error_response(),
        Err(AppError::Domain(m)) if m == BRIDGE_REFUSED => {
            V3Error::from(PolicyError::RoleNotAllowedHere).error_response()
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

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;
    use serde::Deserialize;
    use ts_rs::TS;

    #[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, TS)]
    #[serde(default, rename_all = "camelCase")]
    struct SampleFilter {
        q: String,
        min_amount: Option<u32>,
    }

    impl ViewFilter for SampleFilter {}

    #[test]
    fn parse_filter_returns_static_str_from_serde_fields_slice_without_allocating() {
        let fields = struct_fields::<SampleFilter>().expect("serde fields must be present");
        let min_amount_field = fields
            .iter()
            .find(|&&f| f == "minAmount")
            .copied()
            .expect("minAmount must be in fields");

        let req = TestRequest::default()
            .uri("/?minAmount=not_a_number")
            .to_http_request();
        let err = parse_filter::<SampleFilter>(&req).expect_err("should fail parsing filter");

        assert_eq!(err.code, "invalidRequest");
        let err_field = err.field.expect("field must be present");
        assert_eq!(err_field, "minAmount");

        // Pointer-equality check: must be the exact &'static str from the FIELDS slice
        assert!(
            std::ptr::eq(err_field, min_amount_field),
            "expected err.field to be pointer-equal to the FIELDS slice entry"
        );

        // 10,000 refused requests all return the exact same static pointer without memory growth
        for _ in 0..10_000 {
            let req = TestRequest::default()
                .uri("/?minAmount=bad")
                .to_http_request();
            let err = parse_filter::<SampleFilter>(&req).expect_err("should fail");
            assert!(std::ptr::eq(err.field.unwrap(), min_amount_field));
        }
    }
}
