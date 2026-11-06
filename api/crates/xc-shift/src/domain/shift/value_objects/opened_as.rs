//! The shift a teller opened as: the template chosen on the OPEN count and its business date
//! (configurable-shifts.md §2.3).

use chrono::NaiveDate;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenedAs {
    /// `None` only for a shift opened without a template.
    pub template_id: Option<String>,
    /// The template's name at opening; `Some` exactly when `template_id` is.
    pub template_name: Option<String>,
    /// `Some` for a templated shift. `None` lets the `shifts_business_date_default` trigger date the row by the
    /// branch-local day it opened on.
    pub business_date: Option<NaiveDate>,
}

impl OpenedAs {
    /// A shift opened without a template (only paths that have no teller choice yet).
    pub fn untemplated() -> Self {
        Self::default()
    }
}
