//! Normalisation for PII identity values.

use crate::pii::{IdentityType, PiiError};

/// Normalises an identity value based on its type.
pub fn normalize(
    t: IdentityType,
    raw: &str,
    nationality: Option<&str>,
) -> Result<String, PiiError> {
    match t {
        IdentityType::Passport => {
            let nat = nationality.ok_or(PiiError::InvalidNationality)?;
            let nat = nat.trim();
            if nat.len() != 3 || !nat.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(PiiError::InvalidNationality);
            }
            let num: String = raw
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '-')
                .collect();
            if num.is_empty() {
                return Err(PiiError::Empty);
            }
            Ok(format!(
                "PASSPORT:{}:{}",
                nat.to_ascii_uppercase(),
                num.to_ascii_uppercase()
            ))
        }
        IdentityType::NationalId => {
            let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                return Err(PiiError::Empty);
            }
            Ok(digits)
        }
        IdentityType::Phone => {
            if raw.trim().is_empty() {
                return Err(PiiError::Empty);
            }
            let stripped: String = raw
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '-' && *c != '(' && *c != ')')
                .collect();
            if stripped.is_empty() {
                return Err(PiiError::Empty);
            }
            if let Some(rest) = stripped.strip_prefix('+') {
                if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
                    return Err(PiiError::InvalidPhone);
                }
                Ok(stripped)
            } else if let Some(rest) = stripped.strip_prefix('0') {
                if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
                    return Err(PiiError::InvalidPhone);
                }
                Ok(format!("+66{rest}"))
            } else {
                Err(PiiError::InvalidPhone)
            }
        }
        IdentityType::Email => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(PiiError::Empty);
            }
            if trimmed.contains(char::is_whitespace) {
                return Err(PiiError::InvalidEmail);
            }
            let parts: Vec<&str> = trimmed.split('@').collect();
            if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err(PiiError::InvalidEmail);
            }
            Ok(trimmed.to_lowercase())
        }
    }
}
