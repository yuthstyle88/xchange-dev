//! Command write seam (CM-1, §6).

use std::future::Future;
use std::sync::{Arc, OnceLock};

use actix_web::cookie::Cookie;
use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use pos_infra::transactions::Transactor;
use serde_json::Value;
use sha2::{Digest, Sha256};
use xc_app::domain::app_session::AppSessionId;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::caller::Caller;
use super::error::V3Error;
use super::view::{authenticate_caller, parse_branch, require_branch};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::pg_store::PgIdempotencyStore;
use crate::infrastructure::idempotency::store::{Claim, IdempotencyKey, IdempotencyStore};
use crate::presentation::extractors::app_session::{
    AppSessionCookie, AppSessionGuard, DRAFT_WRITES_PER_MINUTE,
};
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::http::client_address::client_address;

tokio::task_local! {
    static CLIENT_IP: String;
}

pub(crate) fn request_ip() -> Option<String> {
    CLIENT_IP
        .try_with(|ip| {
            let trimmed = ip.trim();
            if trimmed.is_empty() || trimmed.parse::<std::net::IpAddr>().is_err() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .ok()
        .flatten()
}

/// The locks required by a command action (§6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locks {
    One(BranchId),
    Pair(BranchId, BranchId),
    Org,
}

/// How a command uses the anonymous app session `xc_app_sid` (#994 ruling option A; design
/// `customer-app.md` P7, §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppSessionUse {
    /// The seam only reads a session the request carries (APP-18 ruling 4). Every command's default.
    ReadOnly,
    /// For a caller who is not a signed-in customer, the seam reads the request's session or mints
    /// one, admits the write through [`AppSessionGuard`] (`rateLimited` 429), puts the session in
    /// `Caller.app_session`, and answers a newly minted one with its `Set-Cookie`.
    IssueAndLimit,
}

/// A command action that declares its route, method, input and output types, and policy gate (§6).
pub trait CommandAction: Send + Sync + 'static {
    /// The unique operation name stored in `idempotency_records`.
    const NAME: &'static str;
    /// The policy action whose row decides who may call this command.
    const INHERITS: (EntryPath, &'static str);
    type Input: serde::de::DeserializeOwned + Send + 'static;
    type Output: serde::Serialize + Send + 'static;
    /// The exact route served by this command, e.g. `/api/test-drafts/{id}/submit`.
    const ROUTE: &'static str;
    /// The HTTP method for this command.
    const METHOD: Method;
    /// Whether the seam issues and limits the app session for this command; `ReadOnly` by default.
    const APP_SESSION: AppSessionUse = AppSessionUse::ReadOnly;

    /// Returns the locks required by this command action (§6).
    ///
    /// Defaults to `Locks::One(branch.clone())`.
    fn locks(&self, branch: &BranchId, input: &Self::Input) -> Locks {
        let _ = input;
        Locks::One(branch.clone())
    }

    /// What the idempotency record keeps for a replay; the full output by default.
    fn stored_output(&self, output: &Self::Output) -> serde_json::Value {
        serde_json::to_value(output).unwrap_or_default()
    }

    /// Runs the command with authenticated actor, branch, typed input, and idempotency key
    /// within the caller's database transaction.
    fn run<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        who: &'a Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> impl Future<Output = Result<Self::Output, V3Error>> + Send + 'a;

    /// APP-18 #1373 (option B): `run` with the seam's [`Caller`]; the default calls `run` with `caller.user` and
    /// answers `signIn` when there is none.
    fn run_as<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        caller: &'a Caller,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> impl Future<Output = Result<Self::Output, V3Error>> + Send + 'a {
        async move {
            match caller.user.as_ref() {
                Some(who) => self.run(tx, branch, who, input, key).await,
                None => Err(V3Error::from(
                    xc_auth::domain::policy::value_objects::policy_error::PolicyError::SignIn,
                )),
            }
        }
    }
}

/// Helper for commands to map internal failures to `500 serverError`.
/// What a command answers when the caller has no `Standing` (OR-7; formerly the `legacy_actor` bridge, #1030 §3.8):
/// `403 roleNotAllowedHere`, as a refused route does.
pub(crate) fn role_not_allowed_here() -> V3Error {
    V3Error::from(
        xc_auth::domain::policy::value_objects::policy_error::PolicyError::RoleNotAllowedHere,
    )
}

pub fn server_error() -> V3Error {
    V3Error::new(
        "serverError",
        None,
        "เซิร์ฟเวอร์ขัดข้อง",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
}

/// Helper for commands to map scope violations to `403 outOfScope`.
pub fn out_of_scope() -> V3Error {
    V3Error::new(
        "outOfScope",
        None,
        "ไม่มีสิทธิ์ทำรายการสำหรับสาขานี้",
        StatusCode::FORBIDDEN,
    )
}

/// Registers and serves commands at their declared route and method.
pub struct CommandEndpoint;

impl CommandEndpoint {
    pub fn register<C: CommandAction>(cfg: &mut web::ServiceConfig, command: Arc<C>) {
        let route = C::ROUTE;
        let method = C::METHOD;
        cfg.route(
            route,
            web::method(method).to(move |req: HttpRequest, payload: web::Payload| {
                let command = command.clone();
                async move { handle_command(req, payload, command).await }
            }),
        );
    }
}

/// The guard for draft writes: the app's [`AppSessionGuard`] data, else one shared by the process.
fn app_session_guard(req: &HttpRequest) -> &AppSessionGuard {
    static PROCESS_GUARD: OnceLock<AppSessionGuard> = OnceLock::new();
    match req.app_data::<web::Data<AppSessionGuard>>() {
        Some(guard) => guard.get_ref(),
        None => PROCESS_GUARD.get_or_init(|| AppSessionGuard::new(DRAFT_WRITES_PER_MINUTE)),
    }
}

fn current_pii_keys() -> Result<Arc<xc_kernel::pii::PiiKeys>, V3Error> {
    xc_customer::get_pii_keys().map_err(|_| server_error())
}

fn derive_session_id(
    pii_keys: &xc_kernel::pii::PiiKeys,
    org_id: &str,
    command_name: &str,
    idempotency_key: &str,
) -> AppSessionId {
    let subkey = xc_kernel::pii::derive_subkey(pii_keys, b"xc-app-session-v1");
    let mac = xc_kernel::pii::hmac_sha256(
        &subkey,
        &[
            org_id.as_bytes(),
            b"\0",
            command_name.as_bytes(),
            b"\0",
            idempotency_key.as_bytes(),
        ],
    );
    AppSessionId::from_bytes(mac)
}

/// [`AppSessionUse::IssueAndLimit`] for a caller who is not a signed-in customer: the request's
/// session, or a derived one (#1565), admitted by the guard. Answers the `Set-Cookie` of an issued session.
async fn issue_and_limit(
    req: &HttpRequest,
    caller: &mut Caller,
    org_id: &str,
    command_name: &str,
    raw_key: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<Cookie<'static>>, V3Error> {
    if caller
        .user
        .as_ref()
        .is_some_and(|u| u.customer_id.is_some())
    {
        return Ok(None);
    }
    let pii = current_pii_keys()?;
    let derived_id = derive_session_id(&pii, org_id, command_name, raw_key);
    let cookie = AppSessionCookie::for_command(req, derived_id);
    app_session_guard(req).admit_draft_write(&cookie.session().hash, now)?;
    caller.app_session = Some(cookie.session().clone());
    Ok(cookie.issued_cookie())
}

/// The `200` answer of a command, with the app session's `Set-Cookie` when one was issued.
fn ok_answer(body: Value, cookie: Option<Cookie<'static>>) -> HttpResponse {
    let mut answer = HttpResponse::Ok();
    answer
        .insert_header((actix_web::http::header::CONTENT_TYPE, "application/json"))
        .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"));
    if let Some(cookie) = cookie {
        answer.cookie(cookie);
    }
    answer.json(body)
}

async fn handle_command<C: CommandAction>(
    req: HttpRequest,
    payload: web::Payload,
    command: Arc<C>,
) -> HttpResponse {
    // Step 1: Sign-in and Policy::authorize(actor, INHERITS)
    let mut caller = match authenticate_caller(&req, C::INHERITS).await {
        Ok(w) => w,
        Err(res) => return res,
    };

    // Step 2: same_origin check
    if !same_origin(&req) {
        return V3Error::new(
            "originRejected",
            None,
            "ORIGIN_REJECTED",
            StatusCode::BAD_REQUEST,
        )
        .error_response();
    }

    // Step 3: Check Idempotency-Key header
    let raw_key = match req.headers().get("idempotency-key") {
        Some(val) => match val.to_str() {
            Ok(s) if !s.trim().is_empty() => s.to_string(),
            _ => {
                return V3Error::new(
                    "validationFailed",
                    Some("idempotencyKey"),
                    "Idempotency-Key is required",
                    StatusCode::BAD_REQUEST,
                )
                .error_response();
            }
        },
        None => {
            return V3Error::new(
                "validationFailed",
                Some("idempotencyKey"),
                "Idempotency-Key is required",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };
    let idem_key = IdempotencyKey::new(raw_key.clone());

    // Step 4: Body parsing and path parameters
    const MAX_BODY: usize = 65_536;
    let bytes = match payload.to_bytes_limited(MAX_BODY).await {
        Ok(Ok(b)) => b,
        _ => {
            return V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };

    let mut value: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            return V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };

    let obj = match value.as_object_mut() {
        Some(o) => o,
        None => {
            return V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };

    // Insert path parameters from match_info so they are covered by the request hash and Input
    for (name, val) in req.match_info().iter() {
        obj.insert(name.to_string(), Value::String(val.to_string()));
    }

    // Step 5: Check named branch and require_branch
    let named_branch = match obj.get("branch") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => match parse_branch(Some(s.as_str())) {
            Ok(b) => b,
            Err(()) => {
                return V3Error::new(
                    "invalidRequest",
                    Some("branch"),
                    "คำขอไม่ถูกต้อง",
                    StatusCode::BAD_REQUEST,
                )
                .error_response();
            }
        },
        Some(_) => {
            return V3Error::new(
                "invalidRequest",
                Some("branch"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };

    let now = chrono::Utc::now();
    let action_vo = Action::new(C::INHERITS.1);
    let grant = match Policy::authorize(
        caller.user.as_ref(),
        C::INHERITS.0,
        &action_vo,
        named_branch.as_ref(),
        now,
    ) {
        Ok(g) => g,
        Err(e) => return V3Error::from(e).error_response(),
    };
    let branch = grant.branch;
    if let Err(e) = require_branch(&req, &branch).await {
        return e.error_response();
    }

    // Step 6: Validate typed Input before claiming the idempotency key
    let input: C::Input = match serde_json::from_value(value.clone()) {
        Ok(i) => i,
        Err(_) => {
            return V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };

    // Step 6.5: Determine locks and validate scope
    let locks = command.locks(&branch, &input);
    match &locks {
        Locks::One(b) => {
            if b != &branch {
                return out_of_scope().error_response();
            }
        }
        Locks::Pair(a, b) => {
            if a != &branch && b != &branch {
                return out_of_scope().error_response();
            }
        }
        Locks::Org => {}
    }

    let pool = match req.app_data::<web::Data<AppState>>() {
        Some(state) => state.db.clone(),
        None => return server_error().error_response(),
    };

    // Resolve target organisation: for Locks::Org or a customer caller (who has no acting
    // branch; any `branch` field in the body is command data), use the caller's organisation;
    // otherwise resolve from the acting branch and verify the caller belongs to it.
    let org_id: String = match (&locks, caller.user.as_ref()) {
        (Locks::Org, Some(user)) => {
            match sqlx::query_scalar(
                "SELECT o.id::text FROM organizations o WHERE o.id::text = $1 OR o.code = $1",
            )
            .bind(&user.organization_id)
            .fetch_optional(&pool)
            .await
            {
                Ok(Some(org)) => org,
                Ok(None) => return out_of_scope().error_response(),
                Err(e) => {
                    tracing::error!(error = %e, "failed to query caller organization");
                    return server_error().error_response();
                }
            }
        }
        (_, Some(user)) if user.customer_id.is_some() => {
            match sqlx::query_scalar(
                "SELECT o.id::text FROM organizations o WHERE o.id::text = $1 OR o.code = $1",
            )
            .bind(&user.organization_id)
            .fetch_optional(&pool)
            .await
            {
                Ok(Some(org)) => org,
                Ok(None) => return out_of_scope().error_response(),
                Err(e) => {
                    tracing::error!(error = %e, "failed to query caller organization");
                    return server_error().error_response();
                }
            }
        }
        _ => {
            let branch_org_id: String = match sqlx::query_scalar(
                "SELECT organization_id::text FROM branches WHERE id = $1",
            )
            .bind(branch.as_str())
            .fetch_optional(&pool)
            .await
            {
                Ok(Some(org)) => org,
                Ok(None) => return out_of_scope().error_response(),
                Err(e) => {
                    tracing::error!(error = %e, "failed to query branch organization");
                    return server_error().error_response();
                }
            };

            if let Some(user) = caller.user.as_ref() {
                let same_org: bool = match sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM organizations o WHERE o.id::text = $1 AND (o.id::text = $2 OR o.code = $2))",
                )
                .bind(&branch_org_id)
                .bind(&user.organization_id)
                .fetch_one(&pool)
                .await
                {
                    Ok(same) => same,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to check caller organization");
                        return server_error().error_response();
                    }
                };

                if !same_org {
                    return out_of_scope().error_response();
                }
            }

            branch_org_id
        }
    };

    // If Pair, check that the other branch belongs to the same organization
    if let Locks::Pair(a, b) = &locks {
        let other = if a == &branch { b } else { a };
        if other != &branch {
            let other_org_id: String = match sqlx::query_scalar(
                "SELECT organization_id::text FROM branches WHERE id = $1",
            )
            .bind(other.as_str())
            .fetch_optional(&pool)
            .await
            {
                Ok(Some(org)) => org,
                Ok(None) => return out_of_scope().error_response(),
                Err(e) => {
                    tracing::error!(error = %e, "failed to query second branch organization");
                    return server_error().error_response();
                }
            };

            if other_org_id != org_id {
                return out_of_scope().error_response();
            }
        }
    }

    // Step 6.9: The app session, for a command that issues and limits it (#994 option A; #1565)
    let session_cookie = match C::APP_SESSION {
        AppSessionUse::ReadOnly => None,
        AppSessionUse::IssueAndLimit => {
            match issue_and_limit(&req, &mut caller, &org_id, C::NAME, &raw_key, now).await {
                Ok(cookie) => cookie,
                Err(e) => return e.error_response(),
            }
        }
    };

    // Step 7: Request hash & idempotency claim
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap()));

    let store = Arc::new(PgIdempotencyStore::new(pool.clone(), org_id.clone()));

    let claim = match store.claim(C::NAME, &idem_key, &hash).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "idempotency claim failed");
            return server_error().error_response();
        }
    };

    let claim_token = match claim {
        Claim::Conflict => {
            return V3Error::new(
                "idempotencyConflict",
                None,
                "IDEMPOTENCY_CONFLICT",
                StatusCode::CONFLICT,
            )
            .error_response();
        }
        Claim::Replay(answer) => return ok_answer(answer, session_cookie),
        Claim::New(token) => token,
    };

    // Step 8: Run command inside BranchTx and finish claim in the same transaction
    let branch_tx = BranchTx::new(Transactor::new(pool));

    let captured_error = Arc::new(std::sync::Mutex::new(None));
    let captured_error_clone = captured_error.clone();
    let command_clone = command.clone();
    let branch_clone = branch.clone();
    let caller_clone = caller.clone();
    let idem_key_clone = idem_key.clone();
    let claim_token_clone = claim_token.clone();
    let store_clone = store.clone();

    let client_ip = client_address(&req).to_owned();

    let run_res = CLIENT_IP
        .scope(client_ip, async {
            match &locks {
                Locks::One(b) => {
                    branch_tx
                        .run(b, move |tx| {
                            Box::pin(async move {
                                execute_in_tx(
                                    &command_clone,
                                    tx,
                                    &branch_clone,
                                    &caller_clone,
                                    input,
                                    &idem_key_clone,
                                    &claim_token_clone,
                                    &store_clone,
                                    &captured_error_clone,
                                )
                                .await
                            })
                        })
                        .await
                }
                Locks::Pair(a, b) => {
                    branch_tx
                        .run_pair(a, b, move |tx| {
                            Box::pin(async move {
                                execute_in_tx(
                                    &command_clone,
                                    tx,
                                    &branch_clone,
                                    &caller_clone,
                                    input,
                                    &idem_key_clone,
                                    &claim_token_clone,
                                    &store_clone,
                                    &captured_error_clone,
                                )
                                .await
                            })
                        })
                        .await
                }
                Locks::Org => {
                    branch_tx
                        .run_org(&org_id, move |tx| {
                            Box::pin(async move {
                                execute_in_tx(
                                    &command_clone,
                                    tx,
                                    &branch_clone,
                                    &caller_clone,
                                    input,
                                    &idem_key_clone,
                                    &claim_token_clone,
                                    &store_clone,
                                    &captured_error_clone,
                                )
                                .await
                            })
                        })
                        .await
                }
            }
        })
        .await;

    match run_res {
        Ok(response_val) => ok_answer(response_val, session_cookie),
        Err(e) => {
            if let Err(err) = store.release(C::NAME, &idem_key, &claim_token).await {
                tracing::error!(error = %err, "idempotency release failed");
            }
            if let Some(v3_err) = captured_error.lock().unwrap().take() {
                return v3_err.error_response();
            }
            tracing::error!(error = %e, "idempotency finish failed");
            server_error().error_response()
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_in_tx<C: CommandAction>(
    command: &Arc<C>,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    caller: &Caller,
    input: C::Input,
    idem_key: &IdempotencyKey,
    claim_token: &crate::infrastructure::idempotency::store::ClaimToken,
    store: &Arc<PgIdempotencyStore>,
    captured_error: &Arc<std::sync::Mutex<Option<V3Error>>>,
) -> pos_error::AppResult<Value> {
    let output = match command
        .run_as(tx, branch, caller, input, idem_key.clone())
        .await
    {
        Ok(out) => out,
        Err(err) => {
            *captured_error.lock().unwrap() = Some(err);
            return Err(pos_error::AppError::Conflict(
                "command rejected".to_string(),
            ));
        }
    };

    let response_val = match serde_json::to_value(&output) {
        Ok(v) => v,
        Err(e) => {
            return Err(pos_error::AppError::Internal(format!(
                "failed to serialize command output: {e}"
            )));
        }
    };

    store
        .finish_in(
            tx,
            C::NAME,
            idem_key,
            claim_token,
            command.stored_output(&output),
        )
        .await?;

    Ok(response_val)
}
