//! Custody roles.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustodyRole {
    Owner,
    Front,
    Finance,
    Monitor,
    Manager,
    Backend,
    Other,
}
