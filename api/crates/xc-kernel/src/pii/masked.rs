//! Masked representation for PII display and logging.

#[derive(Clone, PartialEq, Eq)]
pub struct Masked(String);

impl Masked {
    pub fn of(raw: &str) -> Self {
        let chars: Vec<char> = raw.chars().collect();
        if chars.len() <= 4 {
            Self("****".to_string())
        } else {
            let last4: String = chars[chars.len() - 4..].iter().collect();
            Self(format!("****{last4}"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Masked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Debug for Masked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
