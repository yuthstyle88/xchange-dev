//! `BenefitError`: the refusals of the budget, coupon and redemption steps (design §4.5, §7, D-8).

use pos_error::AppError;

#[derive(Debug, thiserror::Error)]
pub enum BenefitError {
    /// D-8: an expired reservation could not be re-reserved at completion; the caller rolls back.
    #[error("benefitLimitReached")]
    BenefitLimitReached,
    /// The reservation is not `RESERVED` any more (already consumed or released).
    #[error("staleVersion")]
    StaleReservation,
    #[error("notFound")]
    NotFound,
    #[error(transparent)]
    Store(#[from] AppError),
}

impl BenefitError {
    /// The refusal's code, exactly the `#[error]` text above; `"serverError"` for a store failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BenefitLimitReached => "benefitLimitReached",
            Self::StaleReservation => "staleVersion",
            Self::NotFound => "notFound",
            Self::Store(_) => "serverError",
        }
    }
}

impl From<sqlx::Error> for BenefitError {
    fn from(err: sqlx::Error) -> Self {
        Self::Store(AppError::from(err))
    }
}
