//! Branch contact channels and status (S3 #3406, design `section-b-gaps.md` §S3).

use pos_error::{AppError, AppResult};
use serde::{Deserialize, Serialize};

pub const MAX_CONTACT_CHANNELS: usize = 10;
pub const MAX_CONTACT_VALUE_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContactKind {
    Phone,
    Line,
    Email,
    Facebook,
    Website,
}

impl ContactKind {
    pub fn parse(raw: &str) -> AppResult<Self> {
        match raw {
            "PHONE" => Ok(Self::Phone),
            "LINE" => Ok(Self::Line),
            "EMAIL" => Ok(Self::Email),
            "FACEBOOK" => Ok(Self::Facebook),
            "WEBSITE" => Ok(Self::Website),
            other => Err(AppError::Validation(format!(
                "contactChannels: unknown contact kind {other:?}"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "PHONE",
            Self::Line => "LINE",
            Self::Email => "EMAIL",
            Self::Facebook => "FACEBOOK",
            Self::Website => "WEBSITE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactChannel {
    pub kind: ContactKind,
    pub value: String,
}

impl ContactChannel {
    pub fn new(kind: ContactKind, value: impl AsRef<str>) -> AppResult<Self> {
        let trimmed = value.as_ref().trim();
        let len = trimmed.chars().count();
        if !(1..=MAX_CONTACT_VALUE_CHARS).contains(&len) {
            return Err(AppError::Validation(
                "contactChannels: value must be 1..=200 characters".to_string(),
            ));
        }
        Ok(Self {
            kind,
            value: trimmed.to_string(),
        })
    }

    pub fn parse(kind: &str, value: &str) -> AppResult<Self> {
        let kind = ContactKind::parse(kind)?;
        Self::new(kind, value)
    }

    pub fn validate_list(channels: &[Self]) -> AppResult<Vec<Self>> {
        if channels.len() > MAX_CONTACT_CHANNELS {
            return Err(AppError::Validation(format!(
                "contactChannels: at most {MAX_CONTACT_CHANNELS} channels are allowed"
            )));
        }
        channels
            .iter()
            .map(|ch| Self::new(ch.kind, &ch.value))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BranchStatus {
    #[default]
    Active,
    Inactive,
}

impl BranchStatus {
    pub fn parse(raw: &str) -> AppResult<Self> {
        match raw {
            "ACTIVE" => Ok(Self::Active),
            "INACTIVE" => Ok(Self::Inactive),
            other => Err(AppError::Validation(format!(
                "status: unknown branch status {other:?}"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Inactive => "INACTIVE",
        }
    }
}
