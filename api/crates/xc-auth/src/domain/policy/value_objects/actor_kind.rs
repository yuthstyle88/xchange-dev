//! Actor kind value object.

/// Kind of an actor: owner or staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorKind {
    Owner,
    Staff,
}
