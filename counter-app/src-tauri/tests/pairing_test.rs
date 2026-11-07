//! CA-3c (design §2): the token goes into the store and never crosses to JS. A fake server, no
//! network. Written before the implementation. Do not edit, delete or weaken; add tests in a file
//! of your own.
//!
//! | Pair answer | Store | Answer to the page |
//! |---|---|---|
//! | 200 | holds the token | ids and name; the token appears nowhere in its JSON |
//! | 401 | empty | `PairError::Api(Unauthorized)` |

use xc_counter::keychain::{MemoryTokenStore, TokenStore};
use xc_counter::pairing::{PairError, pair_with};
use xc_device_agent::client::{
    AgentApi, ApiError, ClaimRequest, Claimed, PairAnswer, PairRequest, Report,
};

const TOKEN: &str = "secret-agent-token";

struct FakeServer(Result<PairAnswer, ApiError>);

impl AgentApi for FakeServer {
    async fn pair(&self, _request: &PairRequest) -> Result<PairAnswer, ApiError> {
        self.0.clone()
    }

    async fn claim(&self, _request: &ClaimRequest) -> Result<Option<Claimed>, ApiError> {
        Ok(None)
    }

    async fn report(&self, _report: &Report) -> Result<(), ApiError> {
        Ok(())
    }
}

fn request() -> PairRequest {
    PairRequest {
        code: "ABC123".to_owned(),
        host: "counter-1".to_owned(),
        os: "macos".to_owned(),
        version: "0.1.0".to_owned(),
        build: "abc1234".to_owned(),
    }
}

#[tokio::test]
async fn pairing_stores_the_token_and_never_answers_it() {
    let server = FakeServer(Ok(PairAnswer {
        agent_id: "agent-1".to_owned(),
        token: TOKEN.to_owned(),
        branch_id: "branch-1".to_owned(),
        agent_name: "เคาน์เตอร์ 1".to_owned(),
    }));
    let store = MemoryTokenStore::default();
    let paired = pair_with(&server, &request(), &store)
        .await
        .expect("paired");
    assert_eq!(store.load(), Ok(Some(TOKEN.to_owned())));
    assert_eq!(paired.agent_id, "agent-1");
    let json = serde_json::to_string(&paired).expect("serialises");
    assert!(!json.contains(TOKEN), "the token crossed to JS: {json}");
}

#[tokio::test]
async fn a_refused_pair_stores_nothing() {
    let server = FakeServer(Err(ApiError::Unauthorized));
    let store = MemoryTokenStore::default();
    let answer = pair_with(&server, &request(), &store).await;
    assert_eq!(answer, Err(PairError::Api(ApiError::Unauthorized)));
    assert_eq!(store.load(), Ok(None));
}
