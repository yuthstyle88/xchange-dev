//! Customer display name length limit (#4397): at most 100 Unicode scalar values after trim.

pub const DISPLAY_NAME_MAX_CHARS: usize = 100;

pub fn exceeds_display_name_limit(trimmed: &str) -> bool {
    trimmed.chars().count() > DISPLAY_NAME_MAX_CHARS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_char_limit_boundary() {
        assert!(!exceeds_display_name_limit(&"ก".repeat(100)));
        assert!(exceeds_display_name_limit(&"ก".repeat(101)));
        assert!(exceeds_display_name_limit(&"a".repeat(101)));
    }
}
