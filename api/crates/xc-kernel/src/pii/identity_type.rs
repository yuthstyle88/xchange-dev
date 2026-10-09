//! Identity type enum for PII.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum IdentityType {
    Passport,
    NationalId,
    Phone,
    Email,
}

pub type PiiKind = IdentityType;

impl IdentityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passport => "PASSPORT",
            Self::NationalId => "NATIONAL_ID",
            Self::Phone => "PHONE",
            Self::Email => "EMAIL",
        }
    }
}

impl std::fmt::Display for IdentityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
