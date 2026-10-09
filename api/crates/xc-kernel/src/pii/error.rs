//! PII error types.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PiiError {
    #[error("empty identity value")]
    Empty,
    #[error("invalid nationality")]
    InvalidNationality,
    #[error("invalid phone number")]
    InvalidPhone,
    #[error("invalid email address")]
    InvalidEmail,
    #[error("key file error: {0}")]
    KeyFile(String),
    #[error("decryption failed")]
    Decrypt,
    #[error("unknown key id: {0}")]
    UnknownKey(String),
}
