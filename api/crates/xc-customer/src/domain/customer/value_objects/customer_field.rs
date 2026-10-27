//! The customer field a refusal is about (`docs/design/rpc-refusal-field.md` RF-D2, RF-D5).

use pos_error::AppError;

/// A field of the reserve payload, named by its payload key (RF-D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomerField {
    Name,
    PassportNo,
    Nationality,
    BirthDate,
    PassportExpiry,
    Mode,
}

impl CustomerField {
    /// The payload key, which is also the web form's key.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::PassportNo => "passportNo",
            Self::Nationality => "nationality",
            Self::BirthDate => "birthDate",
            Self::PassportExpiry => "passportExpiry",
            Self::Mode => "mode",
        }
    }
}

/// A refusal of the customer's details that names the field at fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomerRefusal {
    pub field: CustomerField,
    /// v2's sentence, unchanged.
    pub sentence: &'static str,
}

impl From<CustomerRefusal> for AppError {
    fn from(refusal: CustomerRefusal) -> Self {
        AppError::Domain(refusal.sentence.to_owned())
    }
}
