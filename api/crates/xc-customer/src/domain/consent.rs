//! Consent domain types and errors (design `customer-identity-consent.md` §4.1, §4.3).

use std::fmt;
use std::str::FromStr;

use pos_error::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Purpose {
    Terms,
    Marketing,
}

impl Purpose {
    pub fn as_str(&self) -> &'static str {
        match self {
            Purpose::Terms => "TERMS",
            Purpose::Marketing => "MARKETING",
        }
    }
}

impl fmt::Display for Purpose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Purpose {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "TERMS" => Ok(Purpose::Terms),
            "MARKETING" => Ok(Purpose::Marketing),
            _ => Err(format!("unknown purpose: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Channel {
    Sms,
    Email,
    Push,
}

impl Channel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Sms => "SMS",
            Channel::Email => "EMAIL",
            Channel::Push => "PUSH",
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Channel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "SMS" => Ok(Channel::Sms),
            "EMAIL" => Ok(Channel::Email),
            "PUSH" => Ok(Channel::Push),
            _ => Err(format!("unknown channel: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConsentError {
    #[error("ข้อความยินยอมไม่ครบถ้วน")]
    TextMissing,
    #[error("ระยะเวลาจัดเก็บข้อมูลไม่ถูกต้อง")]
    InvalidPeriod,
}

impl ConsentError {
    pub fn code(&self) -> &'static str {
        match self {
            ConsentError::TextMissing => "consentTextMissing",
            ConsentError::InvalidPeriod => "invalidPeriod",
        }
    }
}

impl From<ConsentError> for AppError {
    fn from(e: ConsentError) -> Self {
        AppError::Domain(e.to_string())
    }
}
