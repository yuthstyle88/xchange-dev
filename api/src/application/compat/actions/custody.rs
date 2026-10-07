//! The custody family: every name starting with `ops`, on the staff route.
//!
//! Thirteen actions of the staff page ([`CUSTODY_ACTIONS`]) and whatever other
//! `ops…` name an employee sends. Each is one custody command:
//! `CustodyService::command(branch, action, payload, actor, now)` reads the
//! payload, makes every check (who may, in which phase, whose evidence, which
//! receiver) and writes in one branch transaction. This file adds nothing to
//! those rules; it says who is asking and prints the answer as v2 printed it.
//!
//! Decided, and held by `api/tests/rpc_ops_test.rs`:
//!
//! - the family handles the **staff route only**: v2 knows no `ops*` name on
//!   `/api/rpc`, and `Policy` already refuses them there;
//! - `opsEnable` and `opsSetEntity` are the owner's (`/api/employees`), never
//!   an employee's: `Policy` refuses them on both routes before any family is
//!   asked, and this family does not handle them either;
//! - known difference from v2: an `ops…` name custody does not know is
//!   answered `คำขอไม่ถูกต้อง` whatever the state of the branch; v2 answers it
//!   by that state (`rpc/transport.json` #84, `rpc/ops_actions.json` #2, #23,
//!   #40);
//! - possible difference, not measured in v2: a known name whose payload
//!   cannot be read is refused for the payload before custody looks at the
//!   state (custody off, not clocked in).

use serde_json::Value;
use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::domain::custody::value_objects::custody_actor::CustodyActor;

use crate::application::compat::actions::{ActionFuture, RpcAction, RpcCall};
use crate::application::compat::envelope::RpcRoute;
use crate::application::compat::rpc::RpcCaller;
use crate::application::compat::v2_json::V2Json;
use crate::application::custody_role::custody_role_of;

/// The thirteen custody actions an employee may send, as the staff page and
/// the staff desk name them.
pub const CUSTODY_ACTIONS: [&str; 13] = [
    "opsClock",
    "opsBeginTransaction",
    "opsTarget",
    "opsStartService",
    "opsCutoff",
    "opsPropose",
    "opsHold",
    "opsRelease",
    "opsReject",
    "opsSign",
    "opsDispatch",
    "opsCommit",
    "opsReconcile",
];

/// The two custody commands that are the owner's alone. They come in through
/// `/api/employees`; on the RPC routes `Policy` refuses them for everybody.
const OWNER_ONLY: [&str; 2] = ["opsEnable", "opsSetEntity"];

/// The keys of a custody answer in the order v2 prints them: `{id}`,
/// `{saved, id}` or `{saved}`.
const ANSWER_KEYS: [&str; 2] = ["saved", "id"];

/// The custody family: every name starting with `ops`, on the staff route.
#[derive(Clone)]
pub struct CustodyAction {
    custody: CustodyService,
}

impl CustodyAction {
    /// `custody` runs a custody command of a branch (`CustodyService::command`).
    pub fn new(custody: CustodyService) -> Self {
        Self { custody }
    }
}

impl RpcAction for CustodyAction {
    /// On `/api/staff/rpc`: every name starting with `ops` but the owner's
    /// two. A name custody does not know is handled too — custody refuses it
    /// with `คำขอไม่ถูกต้อง`. On `/api/rpc`: nothing.
    fn handles(&self, route: RpcRoute, action: &str) -> bool {
        route == RpcRoute::StaffRpc && action.starts_with("ops") && !OWNER_ONLY.contains(&action)
    }

    fn run<'a>(&'a self, call: RpcCall<'a>) -> ActionFuture<'a> {
        Box::pin(async move {
            let actor = custody_actor(call.caller);
            let payload = Value::Object(call.payload);
            let answer = self
                .custody
                .command(call.branch, call.action, &payload, &actor, call.now)
                .await?;
            Ok(printed(&answer))
        })
    }
}

/// The caller as custody knows them: the account's id — what attendance,
/// signatures, evidence and the drawer are recorded under, and what the
/// snapshot joins names to — and the part the role plays in custody.
fn custody_actor(caller: &RpcCaller) -> CustodyActor {
    let id = caller.actor.id().to_string();
    let employee_id = match caller.actor.kind() {
        ActorKind::Staff => Some(id.clone()),
        ActorKind::Owner => None,
    };
    CustodyActor {
        id,
        employee_id,
        role: custody_role_of(caller.actor.role()),
    }
}

/// A custody answer with its keys in v2's order: `saved` before `id`
/// (`rpc/ops_actions.json` #54 `{"saved":true,"id":"…"}`). A key custody may
/// add one day comes after them, by name.
fn printed(answer: &Value) -> V2Json {
    let Value::Object(map) = answer else {
        return V2Json::from_value(answer);
    };
    let known = ANSWER_KEYS.iter().filter_map(|key| map.get_key_value(*key));
    let rest = map
        .iter()
        .filter(|(key, _)| !ANSWER_KEYS.contains(&key.as_str()));
    V2Json::Object(
        known
            .chain(rest)
            .map(|(key, value)| (key.clone(), V2Json::from_value(value)))
            .collect(),
    )
}
