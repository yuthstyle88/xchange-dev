//! `TemplateRegistry` — every template by key (design §5). `generated()` is built from
//! `src/templates/*.rs` by `build.rs`; tests build one from their own templates.

use crate::domain::template::NotificationTemplate;

/// Two templates with one key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("two templates have the key {0}")]
pub struct DuplicateTemplateKey(pub String);

pub struct TemplateRegistry {
    templates: Vec<Box<dyn NotificationTemplate>>,
}

impl TemplateRegistry {
    pub fn from_templates(
        templates: Vec<Box<dyn NotificationTemplate>>,
    ) -> Result<Self, DuplicateTemplateKey> {
        let mut seen = std::collections::HashSet::new();
        for t in &templates {
            let key = t.key();
            if !seen.insert(key) {
                return Err(DuplicateTemplateKey(key.to_string()));
            }
        }
        Ok(Self { templates })
    }

    /// The templates of `src/templates/`.
    pub fn generated() -> Self {
        Self::from_templates(crate::templates::generated())
            .expect("generated templates have unique keys")
    }

    pub fn get(&self, key: &str) -> Option<&dyn NotificationTemplate> {
        self.templates
            .iter()
            .find(|t| t.key() == key)
            .map(|b| b.as_ref())
    }
}
