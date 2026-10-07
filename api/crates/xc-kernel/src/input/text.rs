//! STUB (ticket #88).

use super::InputError;

/// A text field: trimmed, counted in UTF-16 units.
pub fn text(raw: &str, max_utf16: usize, required: bool) -> Result<&str, InputError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        if required {
            return Err(InputError::Text);
        }
        return Ok("");
    }
    if trimmed.encode_utf16().count() > max_utf16 {
        return Err(InputError::Text);
    }
    Ok(trimmed)
}
