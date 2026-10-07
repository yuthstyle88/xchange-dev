//! Request and answer shapes of `POST /api/device-setup`, filled by row AG-4 (design §5.4c).

use chrono::SecondsFormat;
use serde_json::{Value, json};

use crate::infrastructure::device_agents::agent_admin::{ListedAgent, PairingCode};

/// Serializes the pairing code response for `add` and `newCode`.
pub fn pairing_code_json(code: &PairingCode) -> Value {
    json!({
        "agentId": code.agent_id,
        "code": code.code,
        "codeExpiresAt": code.code_expires_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

/// Serializes an agent for the `list` response.
pub fn agent_json(agent: &ListedAgent) -> Value {
    json!({
        "agentId": agent.agent_id,
        "branchId": agent.branch_id,
        "name": agent.name,
        "state": agent.state,
        "lastSeenAt": agent.last_seen_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true)),
        "version": agent.version,
        "build": agent.build,
        "os": agent.os,
        "host": agent.host,
        "drawerTransport": agent.drawer_transport,
        "drawerTarget": agent.drawer_target,
        "createdAt": agent.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        "pairedAt": agent.paired_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true)),
        "revokedAt": agent.revoked_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true)),
    })
}

/// Serializes the `list` action response.
pub fn list_response_json(agents: &[ListedAgent]) -> Value {
    json!({
        "agents": agents.iter().map(agent_json).collect::<Vec<_>>(),
    })
}
