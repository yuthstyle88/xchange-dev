//! Publication domain entity and state.

use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PublicationId(Uuid);

impl PublicationId {
    pub fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for PublicationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PublicationId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationState {
    Published,
    Superseded,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Publication {
    pub id: PublicationId,
    pub version: i32,
    pub published_at: chrono::DateTime<chrono::Utc>,
    pub published_by: String,
    pub approved_request_id: String,
    pub draft_id: Option<String>,
}

impl Publication {
    pub fn state(&self, current_version: i32) -> PublicationState {
        if self.version == current_version {
            PublicationState::Published
        } else {
            PublicationState::Superseded
        }
    }
}
