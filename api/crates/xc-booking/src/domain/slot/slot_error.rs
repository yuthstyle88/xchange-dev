//! `SlotError`: the slot refusals of design §4.1 and §4.3, and the store's own failures.

use pos_error::AppError;

#[derive(Debug, thiserror::Error)]
pub enum SlotError {
    /// B3: the reserve's `UPDATE … WHERE status = 'OPEN' AND reserved < capacity` returned no row.
    #[error("CAPACITY_FULL")]
    CapacityFull,
    /// B4: the place to release is no longer held (a lost race).
    #[error("STALE_VERSION")]
    StaleVersion,
    /// §4.1: a slot's capacity can be lowered only to `reserved` or more.
    #[error("capacityBelowReserved")]
    CapacityBelowReserved,
    /// §4.1: a slot with places taken cannot be closed.
    #[error("slotHasBookings")]
    SlotHasBookings,
    /// No such slot (or branch).
    #[error("notFound")]
    NotFound,
    #[error(transparent)]
    Store(#[from] AppError),
}

impl SlotError {
    /// The refusal's code, exactly the `#[error]` text above (`"CAPACITY_FULL"`, `"capacityBelowReserved"`, …);
    /// `"serverError"` for a store failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::CapacityFull => "CAPACITY_FULL",
            Self::StaleVersion => "STALE_VERSION",
            Self::CapacityBelowReserved => "capacityBelowReserved",
            Self::SlotHasBookings => "slotHasBookings",
            Self::NotFound => "notFound",
            Self::Store(_) => "serverError",
        }
    }
}
