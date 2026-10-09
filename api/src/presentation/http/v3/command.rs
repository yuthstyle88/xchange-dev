//! Command write seam (CM-1, §6).

use std::future::Future;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use pos_infra::transactions::Transactor;
use serde_json::Value;
use sha2::{Digest, Sha256};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::error::V3Error;
use super::view::{authenticate_and_authorize, parse_branch, require_branch};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::pg_store::PgIdempotencyStore;
use crate::infrastructure::idempotency::store::{Claim, IdempotencyKey, IdempotencyStore};
use crate::presentation::extractors::origin::same_origin;

/// The locks required by a command action (§6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locks {
    One(BranchId),
    Pair(BranchId, BranchId),
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

    /// Returns the locks required by this command action (§6).
    ///
    /// Defaults to `Locks::One(branch.clone())`.
    fn locks(&self, branch: &BranchId, input: &Self::Input) -> Locks {
        let _ = input;
        Locks::One(branch.clone())
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
}

/// Helper for commands to map internal failures to `500 serverError`.
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

async fn handle_command<C: CommandAction>(
    req: HttpRequest,
    payload: web::Payload,
    command: Arc<C>,
) -> HttpResponse {
    // Step 1: Sign-in and Policy::allow(actor, INHERITS)
    let who = match authenticate_and_authorize(&req, C::INHERITS).await {
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
    let idem_key = IdempotencyKey::new(raw_key);

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

    let branch = Policy::branch(Some(&who), named_branch.as_ref());
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
    }

    let pool = match req.app_data::<web::Data<AppState>>() {
        Some(state) => state.db.clone(),
        None => return server_error().error_response(),
    };

    // Resolve organisation from caller's branch
    let org_id: String =
        match sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
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

    // Step 7: Request hash & idempotency claim
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap()));

    let store = Arc::new(PgIdempotencyStore::new(pool.clone(), org_id));

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
        Claim::Replay(answer) => {
            return HttpResponse::Ok()
                .insert_header((actix_web::http::header::CONTENT_TYPE, "application/json"))
                .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
                .json(answer);
        }
        Claim::New(token) => token,
    };

    // Step 8: Run command inside BranchTx and finish claim in the same transaction
    let branch_tx = BranchTx::new(Transactor::new(pool));

    let captured_error = Arc::new(std::sync::Mutex::new(None));
    let captured_error_clone = captured_error.clone();
    let command_clone = command.clone();
    let branch_clone = branch.clone();
    let who_clone = who.clone();
    let idem_key_clone = idem_key.clone();
    let claim_token_clone = claim_token.clone();
    let store_clone = store.clone();

    let run_res = match &locks {
        Locks::One(b) => {
            branch_tx
                .run(b, move |tx| {
                    Box::pin(async move {
                        execute_in_tx(
                            &command_clone,
                            tx,
                            &branch_clone,
                            &who_clone,
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
                            &who_clone,
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
    };

    match run_res {
        Ok(response_val) => HttpResponse::Ok()
            .insert_header((actix_web::http::header::CONTENT_TYPE, "application/json"))
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(response_val),
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
    who: &Actor,
    input: C::Input,
    idem_key: &IdempotencyKey,
    claim_token: &crate::infrastructure::idempotency::store::ClaimToken,
    store: &Arc<PgIdempotencyStore>,
    captured_error: &Arc<std::sync::Mutex<Option<V3Error>>>,
) -> pos_error::AppResult<Value> {
    let output = match command.run(tx, branch, who, input, idem_key.clone()).await {
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
        .finish_in(tx, C::NAME, idem_key, claim_token, response_val.clone())
        .await?;

    Ok(response_val)
}
