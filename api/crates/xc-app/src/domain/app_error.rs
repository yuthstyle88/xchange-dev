//! The refusals of the customer app's services, by their v3 code (design §4.3, ticket #994 §8).

/// Why a planner or settings call was refused. [`CustomerAppError::code`] is the v3 `code` the
/// command answers (`rates.md` §6 error shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomerAppError {
    /// The organisation has no `customer_app_settings` row, or its draft lifetime is NULL (C4).
    AppSettingsMissing,
    /// An update named an `expectedVersion` that is not the draft's current version.
    StaleVersion,
    /// The draft belongs to another owner, or the caller may not change the settings.
    OutOfScope,
    /// No such draft.
    NotFound,
    /// No current published line for the plan, or its stale policy refuses it (as `QuoteService`).
    RateUnavailable(String),
    /// A field of the input is not acceptable.
    ValidationFailed { field: &'static str },
    /// Anything else (a database failure).
    Server(String),
}

impl From<sqlx::Error> for CustomerAppError {
    fn from(error: sqlx::Error) -> Self {
        CustomerAppError::Server(error.to_string())
    }
}

impl CustomerAppError {
    /// The v3 code: `appSettingsMissing`, `staleVersion`, `outOfScope`, `notFound`,
    /// `rateUnavailable`, `validationFailed` or `serverError`.
    pub fn code(&self) -> &'static str {
        match self {
            CustomerAppError::AppSettingsMissing => "appSettingsMissing",
            CustomerAppError::StaleVersion => "staleVersion",
            CustomerAppError::OutOfScope => "outOfScope",
            CustomerAppError::NotFound => "notFound",
            CustomerAppError::RateUnavailable(_) => "rateUnavailable",
            CustomerAppError::ValidationFailed { .. } => "validationFailed",
            CustomerAppError::Server(_) => "serverError",
        }
    }
}
