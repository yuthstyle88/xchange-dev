//! Client request-id validation: trimmed, 16–60 `[A-Za-z0-9-]` characters (#4137).

use super::{InputError, text};

/// A client request id: `text(raw, 60, true)` first, then 16–60 `[A-Za-z0-9-]` characters.
pub fn request_id(raw: &str) -> Result<&str, InputError> {
    let trimmed = text(raw, 60, true)?;
    if trimmed.len() < 16
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(InputError::RequestId);
    }
    Ok(trimmed)
}
