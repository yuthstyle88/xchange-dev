//! `NotificationTemplate` — one template, one file in `src/templates/` (design P10, §5).

use std::collections::BTreeMap;

/// `notifications.locale` (owner C2: TH/EN/ZH).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    Th,
    En,
    Zh,
}

impl Locale {
    pub fn as_str(&self) -> &'static str {
        match self {
            Locale::Th => "th",
            Locale::En => "en",
            Locale::Zh => "zh",
        }
    }
}

/// Marketing templates need current marketing consent; transactional ones do not
/// (`customer-identity-consent.md` K5, design §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Transactional,
    Marketing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub title: String,
    pub body: String,
}

pub trait NotificationTemplate: Send + Sync {
    fn key(&self) -> &'static str;
    fn version(&self) -> i32;
    fn kind(&self) -> TemplateKind;
    fn render(&self, locale: Locale, params: &BTreeMap<String, String>) -> Rendered;
}
