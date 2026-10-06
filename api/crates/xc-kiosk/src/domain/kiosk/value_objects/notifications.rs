//! What the desk's bell shows.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notifications {
    pub branch_name: String,
    /// Every waiting request of the branch.
    pub count: i64,
    /// The waiting request whose quote is the newest.
    pub latest: Option<String>,
}
