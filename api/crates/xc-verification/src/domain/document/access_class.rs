//! `AccessClass` (design §4.1 `access_class`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessClass {
    IdentityDocument,
}

impl AccessClass {
    /// `IDENTITY_DOCUMENT`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::IdentityDocument => "IDENTITY_DOCUMENT",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "IDENTITY_DOCUMENT" => Some(Self::IdentityDocument),
            _ => None,
        }
    }
}
