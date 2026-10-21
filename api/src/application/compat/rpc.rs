//! CompatRpc: the dispatcher behind `POST /api/rpc` and `POST /api/staff/rpc`.
//!
//! The route layer reads the session and the body; this file does everything
//! after that, in v2's order:
//!
//! 1. may this caller ask for this name on this route — [`Policy::allow`] with
//!    the route's `EntryPath` ([`CompatRpc::allow`]);
//! 2. which branch — an employee's own whatever the payload names, the owner's
//!    `payload.branchId` or `MAIN` — and does it exist;
//! 3. the use case of the action;
//! 4. the answer: the result itself, the snapshot as the six write actions
//!    answer it ([`write_answer`]), and on the staff route the staff view of it
//!    ([`staff_view`] for `adminSnapshot`, [`rewrite_urls`] for everything else).
//!
//! It holds no SQL and no HTTP type. A refusal is a sentence
//! ([`RpcFailure::Refused`]); anything else that goes wrong is handed back for
//! the route to answer as a server error ([`RpcFailure::Failed`]).
//!
//! **Part one** (this file) dispatches six actions itself:
//! `adminSnapshot`, `adminDeskConfig`, `getConnectionStatus`,
//! `adminCreateBranch`, `adminAbandonRequest`, `reserveCustomer`. v2's
//! `adminSaveRate` is retired (RE-7 #604, rates.md §9), and `adminOpenShift`,
//! `adminMoveStock` and `adminCloseShift` are retired (CA-10 #626,
//! `cash-shift-transfer.md` §6): they are answered as names v2 does not know.
//!
//! **Part two** — `saveTransaction` and `documents` — is one file per family
//! under `actions/`, behind [`RpcAction`]. A name that is not one of the six
//! is offered to the registered families ([`CompatRpc::with_actions`]) in the
//! order registered; the first that handles it answers, and steps 1, 2 and 4
//! above stay here. A name no family handles is answered as v2 answers a name
//! it does not know; see [`not_dispatched_yet`].
//!
//! Known differences from v2, each decided before this file was written:
//!
//! - `adminCreateBranch` sent again from another branch than the new one
//!   answers the new branch's snapshot; v2 refuses it with a sentence about
//!   table headers (its own tables for the new branch are not loaded);
//! - what the snapshot leaves out (`row`, `startRow`, `endRow`, `rawStatus`) is
//!   listed in `snapshot.rs`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde_json::{Map, Value};
use xc_auth::domain::access::Standing;
use xc_auth::domain::access::authorizer::Grant;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xc_rate::domain::branch::commands::create_branch::CreateBranch;

use crate::application::compat::abandon_request::AbandonRequest;
use crate::application::compat::actions::{RpcAction, RpcActions, RpcCall};
use crate::application::compat::envelope::{RpcRefusal, RpcRoute};
use crate::application::compat::snapshot::{SnapshotBuilder, write_answer};
use crate::application::compat::staff_view::{StaffMember, rewrite_urls, staff_view};
use crate::application::compat::v2_json::V2Json;

/// `payload.branchId` that is not a branch id at all.
pub const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";

/// The order of the keys the staff view adds to a snapshot (`employees`, then
/// `employee`) and of the keys inside them, as v2 prints them.
const STAFF_KEYS: [&str; 9] = [
    "employees",
    "employee",
    "id",
    "username",
    "name",
    "role",
    "branchId",
    "active",
    "mustChange",
];

/// Who is calling, as the session says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcCaller {
    /// The caller's standing: owner, or staff with role and branch.
    pub standing: Standing,
    /// The account's username: v2's `actor` in a snapshot, and the name every
    /// write is recorded under.
    pub username: String,
    /// The display name: printed in the staff view's `employee`.
    pub name: String,
}

impl RpcCaller {
    fn is_staff(&self) -> bool {
        !self.standing.is_owner()
    }

    /// The caller as the staff view names them; `None` for the owner.
    fn staff_member(&self) -> Option<StaffMember> {
        let branch = self.standing.branch.as_ref()?;
        let role = self.standing.staff_role()?;
        Some(StaffMember {
            id: self.standing.user_id.clone(),
            username: self.username.clone(),
            name: self.name.clone(),
            role,
            role_text: self.standing.kind.audit_text().to_string(),
            branch: branch.clone(),
        })
    }
}

/// Why a request got no result.
#[derive(Debug)]
pub enum RpcFailure {
    /// Refused with a sentence: the caller's doing.
    Refused(RpcRefusal),
    /// Something broke on this side; the route answers a server error and says
    /// nothing of the cause.
    Failed(AppError),
}

impl From<RpcRefusal> for RpcFailure {
    fn from(refusal: RpcRefusal) -> Self {
        RpcFailure::Refused(refusal)
    }
}

impl From<AppError> for RpcFailure {
    fn from(error: AppError) -> Self {
        match error {
            AppError::Domain(sentence) => RpcFailure::Refused(RpcRefusal::refused(sentence)),
            other => RpcFailure::Failed(other),
        }
    }
}

/// The services the six actions of part one are made of.
#[derive(Clone)]
pub struct RpcServices {
    pub branches: BranchService,
    pub customers: CustomerService,
    pub abandon: AbandonRequest,
}

/// The dispatcher of both RPC routes.
#[derive(Clone)]
pub struct CompatRpc {
    services: RpcServices,
    snapshots: SnapshotBuilder,
    actions: RpcActions,
}

impl CompatRpc {
    /// Builds a dispatcher over `services` and `snapshots`.
    pub fn new(services: RpcServices, snapshots: SnapshotBuilder) -> Self {
        Self {
            services,
            snapshots,
            actions: Vec::new(),
        }
    }

    /// The same dispatcher with `actions` as its families of part two, in
    /// place of any it had. The order is the order they are asked in: of two
    /// that handle the same name on the same route, the first answers.
    pub fn with_actions(mut self, actions: RpcActions) -> Self {
        self.actions = actions;
        self
    }

    /// The snapshot builder used by this dispatcher.
    pub fn snapshots(&self) -> &SnapshotBuilder {
        &self.snapshots
    }

    /// Whether `user` may ask for `action` on `route`: `Policy::authorize` with
    /// the route's entry path, the refusal in the route's own sentence.
    ///
    /// The route layer calls this between `RpcRequest::parse` and
    /// `RpcRequest::payload`: v2 judges the name before it looks at the
    /// payload.
    pub fn authorize(
        route: RpcRoute,
        action: &str,
        user: Option<&xc_auth::domain::access::Actor>,
        named: Option<&BranchId>,
        now: DateTime<Utc>,
    ) -> Result<Grant, RpcRefusal> {
        let path = match route {
            RpcRoute::Rpc => EntryPath::Rpc,
            RpcRoute::StaffRpc => EntryPath::StaffRpc,
        };
        Policy::authorize(user, path, &Action::new(action), named, now).map_err(policy_refusal)
    }

    /// Runs `action` for `caller` and gives back what the screen is answered:
    /// print it with `V2Json::to_text`.
    ///
    /// `now` is the server clock at the request. Refusals, in this order: the
    /// role may not ask for the name here; the name is neither one of the six
    /// nor one a registered family handles ([`not_dispatched_yet`]); the
    /// owner's `branchId` is not a branch id (`สาขาไม่ถูกต้อง`); the branch does
    /// not exist (`ไม่พบสาขาที่เลือก กรุณารีเฟรช`); then whatever the use case
    /// or the family says.
    #[allow(clippy::too_many_arguments)]
    pub async fn dispatch(
        &self,
        route: RpcRoute,
        action: &str,
        payload: Map<String, Value>,
        caller: &RpcCaller,
        _session_user: Option<uuid::Uuid>,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> Result<V2Json, RpcFailure> {
        if !BranchId::is_well_formed(branch.as_str()) {
            return Err(RpcRefusal::refused(BAD_BRANCH).into());
        }
        // a name of part two: its family's, when one is registered for it
        let family = if is_part_one(action) {
            None
        } else {
            match self.family_of(route, action) {
                Some(family) => Some(family),
                None => return Err(not_dispatched_yet(route, caller).into()),
            }
        };

        self.services.branches.require(branch).await?;
        let who = caller.username.as_str();

        if let Some(family) = family {
            let call = RpcCall {
                route,
                action,
                payload,
                caller,
                branch,
                now,
            };
            let answer = family.run(call).await.map_err(|err| match err {
                RpcFailure::Failed(AppError::Unauthenticated) => {
                    RpcFailure::Refused(RpcRefusal::not_signed_in(route))
                }
                other => other,
            })?;
            return self.answered(route, action, answer, caller).await;
        }

        let answer = match action {
            "adminSnapshot" => self.snapshots.admin_snapshot(branch, who, now).await?,
            "adminDeskConfig" => self.snapshots.desk_config(branch).await?,
            "getConnectionStatus" => connection_status(),
            "adminCreateBranch" => {
                let command = CreateBranch {
                    request_id: js_text(payload.get("requestId")),
                    name: js_text(payload.get("name")),
                    from_branch: branch.clone(),
                };
                let created = self.services.branches.create(command, who, now).await?;
                // v2 answers with the new branch, not the one the screen was on
                self.written(&created.id, who, now).await?
            }
            "adminAbandonRequest" => {
                let resolution = self
                    .services
                    .abandon
                    .abandon(branch, &Value::Object(payload), who, now)
                    .await?;
                let mut answer = self.written(branch, who, now).await?;
                if let V2Json::Object(pairs) = &mut answer {
                    pairs.push((
                        "requestResolution".to_string(),
                        V2Json::text(resolution.as_str()),
                    ));
                }
                answer
            }
            "reserveCustomer" => {
                let reservation = self
                    .services
                    .customers
                    .reserve(branch, reserve_command(&payload), now)
                    .await?;
                V2Json::object(vec![
                    ("customerId", V2Json::text(reservation.customer_id)),
                    ("existing", V2Json::Bool(reservation.existing)),
                ])
            }
            // `is_part_one` lets no other name through
            _ => return Err(not_dispatched_yet(route, caller).into()),
        };

        self.answered(route, action, answer, caller).await
    }

    /// The first registered family that handles `action` on `route`.
    fn family_of(&self, route: RpcRoute, action: &str) -> Option<&dyn RpcAction> {
        self.actions
            .iter()
            .find(|family| family.handles(route, action))
            .map(|family| family.as_ref())
    }

    /// `answer` as `route` prints it: as it is on `/api/rpc`, the staff view
    /// of it on `/api/staff/rpc`.
    async fn answered(
        &self,
        route: RpcRoute,
        action: &str,
        answer: V2Json,
        caller: &RpcCaller,
    ) -> Result<V2Json, RpcFailure> {
        match route {
            RpcRoute::Rpc => Ok(answer),
            RpcRoute::StaffRpc => Ok(self.staff_answer(action, answer, caller).await?),
        }
    }

    /// The snapshot of `branch` as a write action answers it.
    async fn written(&self, branch: &BranchId, who: &str, now: DateTime<Utc>) -> AppResult<V2Json> {
        let snapshot = self.snapshots.admin_snapshot(branch, who, now).await?;
        Ok(write_answer(&snapshot))
    }

    /// What `/api/staff/rpc` answers where `/api/rpc` answers `answer`: the
    /// staff view of a snapshot, the links rewritten in anything else — with
    /// the keys back in the order v2 prints them.
    async fn staff_answer(
        &self,
        action: &str,
        answer: V2Json,
        caller: &RpcCaller,
    ) -> AppResult<V2Json> {
        let view = match (action, caller.staff_member()) {
            ("adminSnapshot", Some(mut me)) => {
                let colleagues = self.colleagues(&me.branch).await?;
                if let Some(stored) = colleagues.iter().find(|c| c.id == me.id) {
                    me.role_text = stored.role_text.clone();
                }
                staff_view(answer.to_value(), &me, &colleagues)
            }
            _ => rewrite_urls(answer.to_value()),
        };
        Ok(in_the_order_of(view, Some(&answer)))
    }

    /// The active employees of `branch`, oldest account first: the order v2's
    /// list comes in.
    async fn colleagues(&self, branch: &BranchId) -> AppResult<Vec<StaffMember>> {
        crate::infrastructure::compat::snapshot_sources::branch_colleagues(
            self.services.customers.branch_tx().pool(),
            branch,
            Utc::now(),
        )
        .await
    }
}

/// The six actions this file dispatches itself.
fn is_part_one(action: &str) -> bool {
    matches!(
        action,
        "adminSnapshot"
            | "adminDeskConfig"
            | "getConnectionStatus"
            | "adminCreateBranch"
            | "adminAbandonRequest"
            | "reserveCustomer"
    )
}

/// ── NOT DISPATCHED YET — ticket T11a part two ────────────────────────────────
///
/// `saveTransaction` and `documents` pass `Policy` for the callers who may use
/// them, and are answered by a family under `actions/` once that family is
/// wired. Until then — whenever no registered family handles the name — they
/// are answered exactly as v2 answers a name it does not know
/// (`rpc/transport.json` #33, #41, #81): on `/api/rpc` `คำขอไม่ถูกต้อง` to the
/// owner and the role sentence to an employee, on `/api/staff/rpc` the staff
/// route's role sentence. The last ticket of part two deletes this function
/// together with `is_part_one`.
fn not_dispatched_yet(route: RpcRoute, caller: &RpcCaller) -> RpcRefusal {
    match (route, caller.is_staff()) {
        (RpcRoute::Rpc, false) => RpcRefusal::invalid_request(),
        (route, _) => RpcRefusal::role_not_allowed(route),
    }
}

/// A refusal of `Policy` as the routes print it: its status and sentence,
/// `no-store` like every answer but the 401 of `/api/rpc` (which the route
/// layer gives before it ever comes here).
fn policy_refusal(refusal: PolicyError) -> RpcRefusal {
    RpcRefusal {
        status: refusal.status(),
        sentence: refusal.to_string(),
        no_store: true,
    }
}

/// v2's `getConnectionStatus`: the desk asks it once at load and goes on only
/// when `ready` is `true`.
fn connection_status() -> V2Json {
    V2Json::object(vec![
        ("ready", V2Json::Bool(true)),
        ("sheetUrl", V2Json::text("/#transactions")),
        ("folderUrl", V2Json::text("/documents")),
    ])
}

/// `reserveCustomer`'s payload as the customer service takes it. `mode` is
/// passed on only when it is a text: v2 compares it with `LIVE` and `DEMO`
/// without making it a text first.
fn reserve_command(payload: &Map<String, Value>) -> ReserveCustomer {
    ReserveCustomer {
        name: js_text(payload.get("name")),
        passport_no: js_text(payload.get("passportNo")),
        nationality: js_text(payload.get("nationality")),
        birth_date: js_text(payload.get("birthDate")),
        passport_expiry: js_text(payload.get("passportExpiry")),
        mode: payload
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

/// v2's `String(v == null ? "" : v)` for what JSON can hold: nothing and
/// `null` are the empty text, a number its digits, an array its members joined
/// by commas.
fn js_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Array(members)) => members
            .iter()
            .map(|member| js_text(Some(member)))
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}

/// `view` — what the staff view made of an answer, as a `serde_json::Value`,
/// which sorts keys by name — printed with its keys in the order of `like`,
/// the answer it was made from. v2's staff bridge changes values in place and
/// appends `employees` and `employee`, so the order of everything else is the
/// order of the answer; the screens show some objects in key order (the
/// 153-code objects), which a sorted answer would get wrong.
///
/// A key `like` does not have comes after those it has: the staff view's own
/// keys in v2's order ([`STAFF_KEYS`]), anything else by name. A number that
/// was not changed is printed as `like` printed it. A list is matched item by
/// item only while it has kept its length.
fn in_the_order_of(view: Value, like: Option<&V2Json>) -> V2Json {
    match (view, like) {
        (Value::Object(mut map), like) => {
            let mut pairs = Vec::with_capacity(map.len());
            if let Some(V2Json::Object(like_pairs)) = like {
                for (key, like_value) in like_pairs {
                    if let Some(value) = map.remove(key) {
                        pairs.push((key.clone(), in_the_order_of(value, Some(like_value))));
                    }
                }
            }
            let mut rest: BTreeMap<(usize, String), Value> = BTreeMap::new();
            for (key, value) in map {
                let rank = STAFF_KEYS
                    .iter()
                    .position(|known| *known == key)
                    .unwrap_or(STAFF_KEYS.len());
                rest.insert((rank, key), value);
            }
            for ((_, key), value) in rest {
                pairs.push((key, in_the_order_of(value, None)));
            }
            V2Json::Object(pairs)
        }
        (Value::Array(items), Some(V2Json::Array(like_items)))
            if items.len() == like_items.len() =>
        {
            V2Json::Array(
                items
                    .into_iter()
                    .zip(like_items)
                    .map(|(item, like_item)| in_the_order_of(item, Some(like_item)))
                    .collect(),
            )
        }
        (Value::Array(items), _) => V2Json::Array(
            items
                .into_iter()
                .map(|item| in_the_order_of(item, None))
                .collect(),
        ),
        (number @ Value::Number(_), Some(like @ V2Json::Number(_)))
            if like.to_value() == number =>
        {
            like.clone()
        }
        (other, _) => V2Json::from_value(&other),
    }
}
