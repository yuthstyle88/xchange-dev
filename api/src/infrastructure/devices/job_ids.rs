//! Where the id of a new job comes from.

/// Hands out the id of the next job. Tests put their own behind it.
pub trait JobIds: Send + Sync {
    fn next(&self) -> String;
}

/// A random UUID (version 4), lower case with dashes.
#[derive(Debug, Clone, Copy, Default)]
pub struct UuidJobIds;

impl JobIds for UuidJobIds {
    fn next(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}
