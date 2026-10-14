//! Written before the implementation by the test writer gemini/studio-9 (ticket author claude/e92db6). Do not edit, delete or weaken.
//!
//! `NotificationParams` and `DisplayValue` (design `notification-display-values.md` #1629).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;

/// Why a [`DisplayValue`] could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DisplayValueError {
    #[error("invalid queue letter: {0:?}")]
    QueueLetter(char),
    #[error("invalid queue sequence: {0}")]
    QueueSequence(i32),
}

/// A server-produced value that is safe to display in a customer notification without free-text PII checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayValue(pub(crate) String);

impl DisplayValue {
    pub fn queue_number(letter: char, sequence: i32) -> Result<Self, DisplayValueError> {
        if !letter.is_ascii_uppercase() {
            return Err(DisplayValueError::QueueLetter(letter));
        }
        if !(1..=99_999).contains(&sequence) {
            return Err(DisplayValueError::QueueSequence(sequence));
        }
        Ok(Self(queue_number_text(letter, sequence)))
    }

    pub fn count(n: u64) -> Self {
        Self(n.to_string())
    }

    pub fn amount(value: Decimal) -> Self {
        Self(value.to_string())
    }

    pub fn rate(value: Decimal) -> Self {
        Self(value.to_string())
    }

    pub fn date(day: NaiveDate) -> Self {
        Self(day.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Formats a queue number as `{letter}{sequence:03}`.
pub fn queue_number_text(letter: char, sequence: i32) -> String {
    format!("{letter}{sequence:03}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParamValue {
    Text(String),
    Display(DisplayValue),
}

/// Parameters for a notification request, distinguishing between free-text and server-built display values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationParams {
    pub(crate) values: BTreeMap<String, ParamValue>,
}

impl NotificationParams {
    pub fn new() -> Self {
        Self {
            values: BTreeMap::new(),
        }
    }

    pub fn text(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.values
            .insert(key.into(), ParamValue::Text(value.into()));
        self
    }

    pub fn display(mut self, key: impl Into<String>, value: DisplayValue) -> Self {
        self.values.insert(key.into(), ParamValue::Display(value));
        self
    }
}
