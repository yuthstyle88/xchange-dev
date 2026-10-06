//! Actor value object.

use xc_kernel::ids::BranchId;

use super::actor_kind::ActorKind;
use super::role::Role;

/// An actor taking part in an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    id: String,
    kind: ActorKind,
    role: Role,
    branch_id: Option<BranchId>,
}

impl Actor {
    /// Constructs an owner actor.
    pub fn owner(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: ActorKind::Owner,
            role: Role::Owner,
            branch_id: None,
        }
    }

    /// Constructs a staff actor. Returns `None` if `role` is `Role::Owner`.
    pub fn staff(id: impl Into<String>, role: Role, branch: BranchId) -> Option<Self> {
        if role == Role::Owner {
            return None;
        }
        Some(Self {
            id: id.into(),
            kind: ActorKind::Staff,
            role,
            branch_id: Some(branch),
        })
    }

    /// The actor's identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The kind of actor.
    pub fn kind(&self) -> ActorKind {
        self.kind
    }

    /// The actor's role.
    pub fn role(&self) -> Role {
        self.role
    }

    /// The actor's branch, if staff.
    pub fn branch_id(&self) -> Option<&BranchId> {
        self.branch_id.as_ref()
    }
}
