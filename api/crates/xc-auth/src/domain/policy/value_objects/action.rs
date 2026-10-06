//! Action value object.

/// An action being performed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Action(String);

impl Action {
    /// Constructs a new action.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the action name as string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
