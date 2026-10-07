//! The agent bearer-token extractor, filled by row AG-2 (design §5.4c *Identity*).

use std::ops::Deref;

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse};
use xc_kernel::ids::BranchId;

use crate::infrastructure::device_agents::agent_repository::DeviceAgent;

/// An authenticated agent's identity for routes under `/api/device-agent/*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAuth {
    pub agent_id: String,
    pub branch: BranchId,
    pub agent: DeviceAgent,
}

impl AgentAuth {
    pub fn new(agent: DeviceAgent) -> Self {
        Self::from_agent(&agent)
    }

    pub fn from_agent(agent: &DeviceAgent) -> Self {
        Self {
            agent_id: agent.id.clone(),
            branch: BranchId::new(&agent.branch_id),
            agent: agent.clone(),
        }
    }
}

impl Deref for AgentAuth {
    type Target = DeviceAgent;

    fn deref(&self) -> &Self::Target {
        &self.agent
    }
}

/// Checks that the request is not from a browser (ticket §3, design §5.4c).
pub fn check_browser(req: &HttpRequest) -> Option<HttpResponse> {
    if req.headers().contains_key(header::ORIGIN) || req.headers().contains_key(header::COOKIE) {
        return Some(
            HttpResponse::BadRequest()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({ "error": "NOT_FOR_BROWSERS" })),
        );
    }
    None
}

/// Extracts the bearer token from the `Authorization` header.
pub fn extract_bearer_token(req: &HttpRequest) -> Option<&str> {
    let header_val = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    header_val.strip_prefix("Bearer ")
}
