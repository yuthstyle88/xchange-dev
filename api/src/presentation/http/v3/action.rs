//! Compute-only action seam (V0-D, §12.20.3).

use std::future::Future;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use serde_json::Value;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use super::caller::Caller;
use super::error::V3Error;
use super::view::{authenticate_caller, parse_branch, require_branch};
use crate::presentation::extractors::origin::same_origin;

/// A compute-only action that takes typed inputs, computes, and answers without writing (§12.20.3).
pub trait ComputeAction: Send + Sync + 'static {
    /// The route segment and the generated file name: /api/v3/compute/{NAME}, generated/{NAME}.ts.
    const NAME: &'static str;
    /// §4.4: the v2 action whose row decides who may call this one.
    const INHERITS: (EntryPath, &'static str);
    type Request: serde::de::DeserializeOwned + ts_rs::TS + Send;
    type Answer: serde::Serialize + ts_rs::TS + Send;

    /// Computes, reading only. A refusal is the action's own V3Error (code, field, v2's sentence).
    ///
    /// No action's Request may have a field named `branch` (the key is the endpoint's).
    fn compute(
        &self,
        branch: &BranchId,
        who: &Actor,
        request: Self::Request,
    ) -> impl Future<Output = Result<Self::Answer, V3Error>> + Send;

    /// APP-18 #1373 (option B): `compute` with the seam's [`Caller`]; the default calls `compute` with `caller.user`
    /// and answers `signIn` when there is none.
    fn compute_as(
        &self,
        branch: &BranchId,
        caller: &Caller,
        request: Self::Request,
    ) -> impl Future<Output = Result<Self::Answer, V3Error>> + Send {
        async move {
            match caller.user.as_ref() {
                Some(who) => self.compute(branch, who, request).await,
                None => Err(V3Error::from(
                    xc_auth::domain::policy::value_objects::policy_error::PolicyError::SignIn,
                )),
            }
        }
    }
}

/// Helper for actions to map internal failures to `500 serverError` (§12.20.3).
pub fn server_error() -> V3Error {
    V3Error::new(
        "serverError",
        None,
        "เซิร์ฟเวอร์ขัดข้อง",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
}

/// Registers and serves compute actions on `POST /api/v3/compute/{name}`.
pub struct ComputeEndpoint;

impl ComputeEndpoint {
    pub fn register<A: ComputeAction>(cfg: &mut web::ServiceConfig, action: Arc<A>) {
        let path = format!("/api/v3/compute/{}", A::NAME);
        cfg.route(
            &path,
            web::post().to(move |req: HttpRequest, payload: web::Payload| {
                let action = action.clone();
                async move { handle_compute(req, payload, action).await }
            }),
        );
    }
}

async fn handle_compute<A: ComputeAction>(
    req: HttpRequest,
    payload: web::Payload,
    action: Arc<A>,
) -> HttpResponse {
    // Step 1: Sign-in and Policy::authorize(actor, INHERITS)
    let caller = match authenticate_caller(&req, A::INHERITS).await {
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

    // Step 3: Body at most 16,384 bytes, and a JSON object
    const MAX_BODY: usize = 16_384;
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

    let value: Value = match serde_json::from_slice(&bytes) {
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

    let obj = match value.as_object() {
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

    // Step 4: The reserved key branch
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

    // Step 5: Policy::authorize then require_branch
    let now = chrono::Utc::now();
    let action_vo = Action::new(A::INHERITS.1);
    let grant = match Policy::authorize(
        caller.user.as_ref(),
        A::INHERITS.0,
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

    // Step 6: Deserialize value into A::Request
    let request: A::Request = match serde_json::from_value(value) {
        Ok(r) => r,
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

    // Step 7: action.compute_as
    let answer = match action.compute_as(&branch, &caller, request).await {
        Ok(ans) => ans,
        Err(e) => return e.error_response(),
    };

    HttpResponse::Ok()
        .insert_header((actix_web::http::header::CONTENT_TYPE, "application/json"))
        .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
        .json(answer)
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use actix_web::test::TestRequest;
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    use super::*;
    use crate::presentation::pages::{Callers, SessionReader, Sessions};
    use xc_auth::domain::access::{Role, ScopedRole};

    #[derive(Debug, Clone, Deserialize, TS)]
    #[ts(export_to = "test_seam_req.ts")]
    #[allow(dead_code)]
    struct TestReq {
        amount: String,
        #[serde(default)]
        pad: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, TS)]
    #[ts(export_to = "test_seam_ans.ts")]
    #[allow(dead_code)]
    struct TestAns {
        echo: String,
    }

    struct EchoAction;

    impl ComputeAction for EchoAction {
        const NAME: &'static str = "test-seam-unit";
        const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
        type Request = TestReq;
        type Answer = TestAns;

        async fn compute(
            &self,
            _branch: &BranchId,
            _who: &Actor,
            request: TestReq,
        ) -> Result<TestAns, V3Error> {
            Ok(TestAns {
                echo: request.amount,
            })
        }
    }

    struct MockSessions;
    impl SessionReader for MockSessions {
        fn sessions<'a>(
            &'a self,
            _req: &'a HttpRequest,
        ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
            Box::pin(async move {
                Sessions {
                    owner: None,
                    staff: None,
                }
            })
        }

        /// An organisation-wide EXECUTIVE + ADMIN (the former owner, design §3.3).
        fn callers<'a>(
            &'a self,
            _req: &'a HttpRequest,
        ) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
            Box::pin(async move {
                let since = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
                let user = Actor::new(
                    "owner-1",
                    "XC",
                    vec![
                        ScopedRole::new(Role::Executive, None, since, None),
                        ScopedRole::new(Role::Admin, None, since, None),
                    ],
                    None,
                );
                Callers {
                    owner: None,
                    staff: None,
                    user: Some(user),
                    staff_user: None,
                }
            })
        }
    }

    #[actix_web::test]
    async fn branch_sent_as_json_number_refuses_with_field_branch() {
        let app = actix_web::test::init_service(
            actix_web::App::new()
                .configure(|cfg| {
                    ComputeEndpoint::register(cfg, Arc::new(EchoAction));
                })
                .app_data(web::Data::from(
                    Arc::new(MockSessions) as Arc<dyn SessionReader>
                )),
        )
        .await;

        for num in [
            serde_json::json!(1),
            serde_json::json!(0),
            serde_json::json!(42),
            serde_json::json!(12.34),
        ] {
            let req = TestRequest::post()
                .uri("/api/v3/compute/test-seam-unit")
                .insert_header(("host", "xc.test"))
                .insert_header(("origin", "http://xc.test"))
                .insert_header(("x-xchange-request", "1"))
                .set_json(serde_json::json!({
                    "amount": "1",
                    "branch": num,
                }))
                .to_request();
            let resp = actix_web::test::call_service(&app, req).await;
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
            let body: serde_json::Value = actix_web::test::read_body_json(resp).await;
            assert_eq!(body["error"]["code"], "invalidRequest");
            assert_eq!(body["error"]["field"], "branch");
            assert_eq!(body["error"]["message"], "คำขอไม่ถูกต้อง");
        }
    }

    #[actix_web::test]
    async fn body_of_16385_bytes_refused_and_16384_bytes_accepted_at_step_3() {
        let app = actix_web::test::init_service(
            actix_web::App::new()
                .configure(|cfg| {
                    ComputeEndpoint::register(cfg, Arc::new(EchoAction));
                })
                .app_data(web::Data::from(
                    Arc::new(MockSessions) as Arc<dyn SessionReader>
                )),
        )
        .await;

        // 16,385 bytes: refused at step 3 with 400 invalidRequest
        let head = "{\"amount\":\"1\",\"pad\":\"";
        let tail = "\"}";
        let pad_over = "x".repeat(16385 - head.len() - tail.len());
        let body_over = format!("{head}{pad_over}{tail}");
        assert_eq!(body_over.len(), 16385);

        let req = TestRequest::post()
            .uri("/api/v3/compute/test-seam-unit")
            .insert_header(("host", "xc.test"))
            .insert_header(("origin", "http://xc.test"))
            .insert_header(("x-xchange-request", "1"))
            .set_payload(body_over)
            .to_request();
        let resp = actix_web::test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = actix_web::test::read_body_json(resp).await;
        assert_eq!(body["error"]["code"], "invalidRequest");
        assert_eq!(body["error"]["field"], serde_json::Value::Null);

        // 16,384 bytes: accepted at step 3 (proceeds past step 3 and 4, reaching step 5 require_branch)
        let pad_exact = "x".repeat(16384 - head.len() - tail.len());
        let body_exact = format!("{head}{pad_exact}{tail}");
        assert_eq!(body_exact.len(), 16384);

        let req = TestRequest::post()
            .uri("/api/v3/compute/test-seam-unit")
            .insert_header(("host", "xc.test"))
            .insert_header(("origin", "http://xc.test"))
            .insert_header(("x-xchange-request", "1"))
            .set_payload(body_exact)
            .to_request();
        let resp = actix_web::test::call_service(&app, req).await;
        // Step 3 did NOT refuse!
        assert_ne!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
