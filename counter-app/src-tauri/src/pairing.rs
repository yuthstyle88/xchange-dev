//! CA-3c. First run: the pair code becomes an agent token (design §2 "Pairing reuses the agent's").
//! The token goes into the [`TokenStore`] and never crosses to JS, which is why [`Paired`] has no
//! token field.

use serde::Serialize;
use url::Url;
use xc_device_agent::client::{AgentApi, ApiError, HttpAgentApi, PairRequest};
use xc_device_agent::config::{BUILD, VERSION, current_host, current_os};

use crate::keychain::{TokenError, TokenStore};

/// What the pair page is told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paired {
    pub agent_id: String,
    pub branch_id: String,
    pub agent_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairError {
    Api(ApiError),
    Store(TokenError),
}

impl std::fmt::Display for PairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(err) => write!(f, "{err}"),
            Self::Store(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PairError {}

/// Pairs through `xc_device_agent::client::HttpAgentApi::for_pairing`, then [`pair_with`].
pub async fn pair(server: &Url, code: &str, tokens: &dyn TokenStore) -> Result<Paired, PairError> {
    let api = HttpAgentApi::for_pairing(server.as_str()).map_err(PairError::Api)?;
    let request = PairRequest {
        code: code.to_owned(),
        host: current_host(),
        os: current_os(),
        version: VERSION.to_owned(),
        build: BUILD.to_owned(),
    };
    pair_with(&api, &request, tokens).await
}

/// Sends `request` to `api` and stores the answer's token. On any failure nothing is stored.
pub async fn pair_with<A: AgentApi>(
    api: &A,
    request: &PairRequest,
    tokens: &dyn TokenStore,
) -> Result<Paired, PairError> {
    let answer = api.pair(request).await.map_err(PairError::Api)?;
    tokens.save(&answer.token).map_err(PairError::Store)?;
    Ok(Paired {
        agent_id: answer.agent_id,
        branch_id: answer.branch_id,
        agent_name: answer.agent_name,
    })
}
